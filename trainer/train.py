#!/usr/bin/env python3
"""Pinned, offline PyMC trainer for false-green calibration.

Run only through:
  uv run --frozen --python 3.12 python trainer/train.py --data ... --out ...

The Rust engine verifies the signed input dataset and independently recomputes
release metrics before it signs a runtime bundle. This process never publishes
or uploads data.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
from collections import defaultdict
from pathlib import Path
from typing import Any

# Pin numerical libraries to one thread before importing NumPy/PyMC.
for variable in (
    "OMP_NUM_THREADS",
    "OPENBLAS_NUM_THREADS",
    "MKL_NUM_THREADS",
    "VECLIB_MAXIMUM_THREADS",
    "NUMEXPR_NUM_THREADS",
):
    os.environ[variable] = "1"

import blake3
import numpy as np
import pymc as pm
import pytensor.tensor as pt

SEED = 20260714
NUMERIC_FEATURES = (
    "verification_strength",
    "attempts",
    "flakiness",
    "change_size",
)
CATEGORICAL_FEATURES = (
    "agent_variant",
    "task_family",
    "repository_id",
    "language",
    "freshness",
    "environment_match",
    "negative_control_status",
    "verifier_independent",
)


def digest(value: str) -> str:
    return blake3.blake3(value.encode("utf-8")).hexdigest()


def observation_key(row: dict[str, Any]) -> str:
    return str(row["observation_key"])


def cluster_rows(rows: list[dict[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[observation_key(row)].append(row)
    clustered = []
    for key in sorted(grouped):
        members = grouped[key]
        total_weight = sum(float(item["weight"]) for item in members)
        cluster_weight = max(float(item["weight"]) for item in members)
        failures = sum(
            (1.0 if item["result"] == "failure" else 0.0) * float(item["weight"])
            for item in members
        )
        row = dict(members[0])
        row["failure"] = failures / total_weight
        row["weight"] = cluster_weight
        for feature in NUMERIC_FEATURES:
            row[feature] = sum(float(item[feature]) for item in members) / len(members)
        row["observation_key"] = key
        clustered.append(row)
    return clustered


def heldout_keys(rows: list[dict[str, Any]]) -> tuple[str, set[str]]:
    repositories = sorted({row["repository_id"] for row in rows})
    if len(repositories) >= 5:
        selected = {value for value in repositories if int(digest(value)[:2], 16) < 51}
        if not selected:
            selected = {min(repositories, key=digest)}
        return "held-out-repositories", {
            row["observation_key"] for row in rows if row["repository_id"] in selected
        }
    tasks = sorted({row["task_id"] for row in rows})
    selected_tasks = {value for value in tasks if int(digest(value)[:2], 16) < 51}
    if not selected_tasks and tasks:
        selected_tasks = {min(tasks, key=digest)}
    return "held-out-tasks", {
        row["observation_key"] for row in rows if row["task_id"] in selected_tasks
    }


def categorical_value(row: dict[str, Any], feature: str) -> str:
    if feature == "agent_variant":
        cohort = row["cohort"]
        return f'{cohort["provider"]}:{cohort["model_snapshot"]}:{cohort["agent_name"]}:{cohort["agent_version"]}'
    if feature == "task_family":
        return str(row["cohort"]["task_family"])
    if feature == "verifier_independent":
        return "true" if row[feature] else "false"
    return str(row[feature])


def encode(
    train: list[dict[str, Any]], heldout: list[dict[str, Any]]
) -> tuple[np.ndarray, np.ndarray, dict[str, tuple[float, float]], dict[str, list[str]], dict[str, tuple[np.ndarray, np.ndarray]]]:
    raw_train = np.asarray([[float(row[name]) for name in NUMERIC_FEATURES] for row in train])
    means = raw_train.mean(axis=0)
    scales = raw_train.std(axis=0)
    scales[scales < 1.0e-9] = 1.0
    raw_heldout = np.asarray([[float(row[name]) for name in NUMERIC_FEATURES] for row in heldout])
    train_x = (raw_train - means) / scales
    heldout_x = (raw_heldout - means) / scales
    scaling = {
        name: (float(means[index]), float(scales[index]))
        for index, name in enumerate(NUMERIC_FEATURES)
    }
    domains: dict[str, list[str]] = {}
    indices: dict[str, tuple[np.ndarray, np.ndarray]] = {}
    for feature in CATEGORICAL_FEATURES:
        domain = sorted({categorical_value(row, feature) for row in train})
        domains[feature] = domain
        lookup = {value: index for index, value in enumerate(domain)}
        train_index = np.asarray([lookup[categorical_value(row, feature)] for row in train])
        heldout_index = np.asarray(
            [lookup.get(categorical_value(row, feature), -1) for row in heldout]
        )
        indices[feature] = (train_index, heldout_index)
    return train_x, heldout_x, scaling, domains, indices


def flatten_samples(idata: Any, name: str) -> np.ndarray:
    values = np.asarray(idata.posterior[name])
    return values.reshape((-1,) + values.shape[2:])


def predict_draws(
    idata: Any,
    numeric: np.ndarray,
    indices: dict[str, np.ndarray],
) -> np.ndarray:
    intercept = flatten_samples(idata, "intercept")
    beta = flatten_samples(idata, "beta_numeric")
    logits = intercept[:, None] + beta @ numeric.T
    for feature in CATEGORICAL_FEATURES:
        effects = flatten_samples(idata, f"effect_{feature}")
        index = indices[feature]
        known = index >= 0
        if known.any():
            logits[:, known] += effects[:, index[known]]
    return 1.0 / (1.0 + np.exp(-np.clip(logits, -30.0, 30.0)))


def calibration_slope(actual: np.ndarray, predicted: np.ndarray, weights: np.ndarray) -> float:
    logits = np.log(np.clip(predicted, 1.0e-6, 1.0 - 1.0e-6) / np.clip(1.0 - predicted, 1.0e-6, 1.0))
    design = np.column_stack((np.ones_like(logits), logits))
    coefficients = np.asarray([0.0, 1.0])
    for _ in range(50):
        fitted = 1.0 / (1.0 + np.exp(-np.clip(design @ coefficients, -30.0, 30.0)))
        gradient = design.T @ (weights * (actual - fitted))
        variance = weights * fitted * (1.0 - fitted)
        hessian = -(design.T * variance) @ design
        try:
            step = np.linalg.solve(hessian, gradient)
        except np.linalg.LinAlgError:
            return 0.0
        coefficients -= step
        if np.max(np.abs(step)) < 1.0e-8:
            break
    return float(coefficients[1])


def metrics(
    actual: np.ndarray,
    predicted: np.ndarray,
    weights: np.ndarray,
    base_predictions: np.ndarray,
) -> dict[str, float]:
    total = float(weights.sum())
    brier = float(np.sum(weights * (predicted - actual) ** 2) / total)
    base_brier = float(np.sum(weights * (base_predictions - actual) ** 2) / total)
    improvement = (base_brier - brier) / base_brier if base_brier > 0 else 0.0
    ece = 0.0
    for lower in np.linspace(0.0, 0.9, 10):
        mask = (predicted >= lower) & (predicted < lower + 0.1)
        if mask.any():
            bin_weight = float(weights[mask].sum())
            observed = float(np.sum(weights[mask] * actual[mask]) / bin_weight)
            forecast = float(np.sum(weights[mask] * predicted[mask]) / bin_weight)
            ece += bin_weight / total * abs(observed - forecast)
    return {
        "brier_score": brier,
        "cohort_base_rate_brier": base_brier,
        "brier_improvement_fraction": improvement,
        "expected_calibration_error": ece,
        "calibration_slope": calibration_slope(actual, predicted, weights),
    }


def train(data_path: Path, out_path: Path, draws: int, tune: int) -> None:
    source = json.loads(data_path.read_text(encoding="utf-8"))
    if source.get("record_kind") != "calibration_dataset":
        raise ValueError("input is not a Receipts calibration dataset")
    rows = cluster_rows(source["observations"])
    if len(rows) < 2:
        raise ValueError("hierarchical trainer requires at least two clustered observations")
    split_kind = source["split_kind"]
    train_rows = [row for row in rows if row["split"] == "train"]
    heldout_rows = [row for row in rows if row["split"] == "heldout"]
    if not train_rows or not heldout_rows:
        raise ValueError("deterministic split must have both training and held-out observations")
    train_x, heldout_x, scaling, domains, all_indices = encode(train_rows, heldout_rows)
    train_y = np.asarray([row["failure"] for row in train_rows])
    train_weights = np.asarray([float(row["weight"]) for row in train_rows])

    with pm.Model() as model:
        intercept = pm.Normal("intercept", mu=0.0, sigma=1.5)
        beta_numeric = pm.Normal("beta_numeric", mu=0.0, sigma=1.0, shape=len(NUMERIC_FEATURES))
        logits = intercept + pt.dot(train_x, beta_numeric)
        for feature in CATEGORICAL_FEATURES:
            sigma = pm.HalfNormal(f"sigma_{feature}", sigma=1.0)
            effect = pm.Normal(
                f"effect_{feature}", mu=0.0, sigma=sigma, shape=len(domains[feature])
            )
            logits = logits + effect[all_indices[feature][0]]
        probability = pm.math.sigmoid(logits)
        logp = train_y * pm.math.log(probability) + (1.0 - train_y) * pm.math.log(1.0 - probability)
        pm.Potential("weighted_observations", pt.sum(train_weights * logp))
        idata = pm.sample(
            draws=draws,
            tune=tune,
            chains=2,
            cores=1,
            random_seed=SEED,
            progressbar=False,
            compute_convergence_checks=False,
            return_inferencedata=True,
        )

    heldout_indices = {name: values[1] for name, values in all_indices.items()}
    heldout_draws = predict_draws(idata, heldout_x, heldout_indices)
    point_predictions = heldout_draws.mean(axis=0)
    actual = np.asarray([row["failure"] for row in heldout_rows])
    weights = np.asarray([float(row["weight"]) for row in heldout_rows])
    def cohort_key(row: dict[str, Any]) -> tuple[str, ...]:
        cohort = row["cohort"]
        return (
            cohort["provider"], cohort["model_snapshot"], cohort["agent_name"],
            cohort["agent_version"], cohort["task_family"],
        )

    cohort_totals: dict[tuple[str, ...], list[float]] = defaultdict(lambda: [0.0, 0.0])
    for row in train_rows:
        cohort_totals[cohort_key(row)][0] += float(row["failure"]) * float(row["weight"])
        cohort_totals[cohort_key(row)][1] += float(row["weight"])
    global_rate = float(np.sum(train_weights * train_y) / np.sum(train_weights))
    base_predictions = np.asarray([
        cohort_totals[cohort_key(row)][0] / cohort_totals[cohort_key(row)][1]
        if cohort_key(row) in cohort_totals else global_rate
        for row in heldout_rows
    ])
    measured = metrics(actual, point_predictions, weights, base_predictions)
    posterior_draws = np.average(heldout_draws, axis=1, weights=weights)

    parameters: dict[str, list[float]] = {
        "intercept": flatten_samples(idata, "intercept").tolist(),
    }
    numeric_samples = flatten_samples(idata, "beta_numeric")
    for index, name in enumerate(NUMERIC_FEATURES):
        parameters[f"numeric:{name}"] = numeric_samples[:, index].tolist()
    for feature in CATEGORICAL_FEATURES:
        samples = flatten_samples(idata, f"effect_{feature}")
        for index, value in enumerate(domains[feature]):
            parameters[f"{feature}:{value}"] = samples[:, index].tolist()

    output = {
        "format_version": "1",
        "methodology_version": "hierarchical-logistic-v1",
        "dataset_hash": source["dataset_hash"],
        "seed": SEED,
        "single_threaded": True,
        "chains": 2,
        "draws_per_chain": draws,
        "tune_per_chain": tune,
        "python_version": platform.python_version(),
        "pymc_version": pm.__version__,
        "numpy_version": np.__version__,
        "split_kind": split_kind,
        "training_observation_keys": sorted(row["observation_key"] for row in train_rows),
        "held_out_predictions": [
            {
                "observation_key": row["observation_key"],
                "actual_failure": float(row["failure"]),
                "predicted_failure": float(point_predictions[index]),
                "weight": float(row["weight"]),
            }
            for index, row in enumerate(heldout_rows)
        ],
        "metrics": measured,
        "posterior_draws": posterior_draws.tolist(),
        "feature_scaling": {
            name: {"mean": mean, "scale": scale} for name, (mean, scale) in scaling.items()
        },
        "feature_domains": domains,
        "model_parameters": parameters,
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(output, sort_keys=True, separators=(",", ":")) + "\n", encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--draws", type=int, default=1000)
    parser.add_argument("--tune", type=int, default=1000)
    args = parser.parse_args()
    if args.draws < 10 or args.tune < 10:
        parser.error("--draws and --tune must be at least 10")
    train(args.data, args.out, args.draws, args.tune)


if __name__ == "__main__":
    main()
