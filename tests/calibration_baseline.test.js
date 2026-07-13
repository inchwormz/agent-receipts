import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cli = path.join(repoRoot, "bin", "receipts.mjs");

function command(args) {
  return spawnSync(process.execPath, [cli, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

test("zero eligible outcomes produces a signed insufficient-data bundle with no probability", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-calibration-empty-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const runs = path.join(fixture, "runs");
  const bundle = path.join(fixture, "calibration.bundle.json");
  fs.mkdirSync(runs);

  const result = command(["calibration", "build", "--runs", runs, "--out", bundle]);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  const report = JSON.parse(result.stdout);
  assert.equal(report.publication_state, "insufficient_data");
  assert.equal(report.effective_outcomes, 0);
  const saved = JSON.parse(fs.readFileSync(bundle, "utf8"));
  assert.equal(saved.publication_state, "insufficient_data");
  assert.equal(saved.posterior.false_green_probability, null);
  assert.equal(saved.posterior.upper_95_false_green_risk, null);
  assert.match(saved.dataset_hash, /^[0-9a-f]{64}$/);
  assert.match(saved.signature, /^[0-9a-f]{128}$/);

  const verify = command(["calibration", "verify", "--bundle", bundle]);
  assert.equal(verify.status, 0, `${verify.stdout}\n${verify.stderr}`);
  const tampered = { ...saved, publication_state: "calibrated" };
  fs.writeFileSync(bundle, `${JSON.stringify(tampered)}\n`);
  const afterTamper = command(["calibration", "verify", "--bundle", bundle]);
  assert.notEqual(afterTamper.status, 0, "changed calibration bundle must fail closed");
});

test("external task results are fractional, repeated tasks cluster, and model cards add zero outcomes", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-calibration-imports-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const runs = path.join(fixture, "runs");
  const imports = path.join(fixture, "imports");
  const bundle = path.join(fixture, "calibration.bundle.json");
  const datasetFile = path.join(fixture, "calibration.dataset.json");
  fs.mkdirSync(runs);

  const base = {
    format_version: "1",
    data_kind: "task-results",
    source_url: "https://example.invalid/eval/v1",
    retrieval_date: "2026-07-14",
    methodology_version: "method-v1",
    harness_version: "harness-v2",
    sample_size: 2,
    attribution: "Fixture Evaluation Authors",
    license: "CC-BY-4.0",
    records: [
      { task_id: "task-1", result: "success", provider: "openai", model_snapshot: "gpt-5-2026-07-01", agent_name: "codex", agent_version: "1.2.3", task_family: "bugfix", repository_id: "repo-a", language: "rust" },
      { task_id: "task-2", result: "failure", provider: "openai", model_snapshot: "gpt-5-2026-07-01", agent_name: "codex", agent_version: "1.2.3", task_family: "bugfix", repository_id: "repo-b", language: "rust" },
    ],
  };
  const importFile = (name, value) => {
    const file = path.join(fixture, name);
    fs.writeFileSync(file, `${JSON.stringify(value)}\n`);
    const result = command(["import-eval", "--from", file, "--out", imports]);
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  };
  importFile("tasks-a.json", base);
  importFile("tasks-b.json", { ...base, source_url: "https://example.invalid/eval/v2" });
  importFile("model-card.json", {
    ...base,
    data_kind: "model-card-metadata",
    source_url: "https://example.invalid/model-card/v1",
  });

  const result = command([
    "calibration", "build", "--runs", runs, "--imports", imports, "--out", bundle,
  ]);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  const saved = JSON.parse(fs.readFileSync(bundle, "utf8"));
  assert.equal(saved.sample_counts.raw_outcomes, 4);
  assert.equal(saved.sample_counts.eligible_outcomes, 4);
  assert.equal(saved.sample_counts.effective_outcomes, 0.5);
  assert.equal(saved.sample_counts.clustered_repeats, 2);
  assert.equal(saved.source_dataset_hashes.length, 3);
  assert.equal(saved.cohorts[0].effective_outcomes, 0.5);
  assert.equal(saved.cohorts[0].posterior.false_green_probability, null);

  const datasetResult = command([
    "calibration", "dataset", "--runs", runs, "--imports", imports, "--out", datasetFile,
  ]);
  assert.equal(datasetResult.status, 0, `${datasetResult.stdout}\n${datasetResult.stderr}`);
  const dataset = JSON.parse(fs.readFileSync(datasetFile, "utf8"));
  assert.equal(dataset.dataset_hash, saved.dataset_hash);
  assert.match(dataset.signature, /^[0-9a-f]{128}$/);
  assert.ok(dataset.observations.every((row) => ["train", "heldout"].includes(row.split)));

  const draws = Array(2000).fill(0);
  const trainerOutput = path.join(fixture, "trainer-output.json");
  fs.writeFileSync(trainerOutput, `${JSON.stringify({
    format_version: "1",
    methodology_version: "hierarchical-logistic-v1",
    dataset_hash: dataset.dataset_hash,
    seed: 20260714,
    single_threaded: true,
    chains: 2,
    draws_per_chain: 1000,
    tune_per_chain: 1000,
    python_version: "3.12.10",
    pymc_version: "5.27.1",
    numpy_version: "2.3.5",
    split_kind: dataset.split_kind,
    training_observation_keys: [],
    held_out_predictions: [],
    metrics: {
      brier_score: 0,
      cohort_base_rate_brier: 0,
      brier_improvement_fraction: 0,
      expected_calibration_error: 0,
      calibration_slope: 0,
    },
    posterior_draws: Array(2000).fill(0.5),
    feature_scaling: {
      verification_strength: { mean: 0, scale: 1 },
      attempts: { mean: 0, scale: 1 },
      flakiness: { mean: 0, scale: 1 },
      change_size: { mean: 0, scale: 1 },
    },
    feature_domains: {},
    model_parameters: {
      intercept: draws,
      "numeric:verification_strength": draws,
      "numeric:attempts": draws,
      "numeric:flakiness": draws,
      "numeric:change_size": draws,
    },
  })}\n`);
  const promote = command([
    "calibration", "promote", "--dataset", datasetFile, "--trainer-output", trainerOutput,
    "--lock", path.join(repoRoot, "uv.lock"), "--out", path.join(fixture, "hierarchical.bundle.json"),
  ]);
  assert.notEqual(promote.status, 0);
  assert.match(promote.stderr, /hierarchical release ineligible/);
});

test("thirty effective outcomes expose only a provisional cohort posterior", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-calibration-provisional-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const runs = path.join(fixture, "runs");
  const imports = path.join(fixture, "imports");
  const source = path.join(fixture, "tasks.json");
  const bundle = path.join(fixture, "calibration.bundle.json");
  fs.mkdirSync(runs);
  const records = Array.from({ length: 120 }, (_, index) => ({
    task_id: `task-${index}`,
    result: index % 4 === 0 ? "failure" : "success",
    provider: "openai",
    model_snapshot: "gpt-5-2026-07-01",
    agent_name: "codex",
    agent_version: "1.2.3",
    task_family: "bugfix",
    repository_id: `repo-${index % 10}`,
    language: "rust",
  }));
  fs.writeFileSync(source, `${JSON.stringify({
    format_version: "1",
    data_kind: "task-results",
    source_url: "https://example.invalid/eval/provisional-v1",
    retrieval_date: "2026-07-14",
    methodology_version: "method-v1",
    harness_version: "harness-v2",
    sample_size: records.length,
    attribution: "Fixture Evaluation Authors",
    license: "CC-BY-4.0",
    records,
  })}\n`);
  assert.equal(command(["import-eval", "--from", source, "--out", imports]).status, 0);
  const result = command([
    "calibration", "build", "--runs", runs, "--imports", imports, "--out", bundle,
  ]);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  const saved = JSON.parse(fs.readFileSync(bundle, "utf8"));
  assert.equal(saved.publication_state, "provisional");
  assert.equal(saved.sample_counts.effective_outcomes, 30);
  assert.equal(saved.cohorts[0].publication_state, "provisional");
  assert.equal(saved.cohorts[0].calibration_status, "held_out_metrics_unavailable");
  assert.equal(typeof saved.cohorts[0].posterior.false_green_probability, "number");
  assert.equal(typeof saved.cohorts[0].posterior.upper_95_false_green_risk, "number");
});
