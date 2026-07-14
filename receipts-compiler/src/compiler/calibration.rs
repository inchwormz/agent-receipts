//! Runtime calibration baseline and signed bundle verification.
//!
//! The baseline is deliberately conservative: exact model-agent-task-family
//! cohorts use a Beta(1,1) prior, repeated task IDs contribute at most one
//! effective observation, and no probability is exposed below 30 effective
//! independently adjudicated outcomes.

use crate::compiler::crypto::{
    ExecutorIdentity, SignedEngineIdentity, current_engine_identity, current_executor_identity,
    sign_detached, verify_detached,
};
use crate::compiler::imports::load_verified_imports;
use crate::compiler::outcomes::{IndependentOutcome, load_outcome_records};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const BUNDLE_HASH_DOMAIN: &[u8] = b"agent-receipts:v1:calibration-bundle-hash\0";
const BUNDLE_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:v1:calibration-bundle";
const DATASET_HASH_DOMAIN: &[u8] = b"agent-receipts:v1:calibration-dataset-hash\0";
const DATASET_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:v1:calibration-dataset";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PosteriorSummary {
    pub prior_alpha: f64,
    pub prior_beta: f64,
    pub posterior_alpha: Option<f64>,
    pub posterior_beta: Option<f64>,
    pub false_green_probability: Option<f64>,
    pub upper_95_false_green_risk: Option<f64>,
    pub interval_95_width: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct CohortIdentity {
    pub provider: String,
    pub model_snapshot: String,
    pub agent_name: String,
    pub agent_version: String,
    pub task_family: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CohortCalibration {
    pub cohort: CohortIdentity,
    pub raw_outcomes: u64,
    pub effective_outcomes: f64,
    pub false_green_events: f64,
    pub publication_state: String,
    pub calibration_status: String,
    pub posterior: PosteriorSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SampleCounts {
    pub raw_outcomes: u64,
    pub eligible_outcomes: u64,
    pub effective_outcomes: f64,
    pub clustered_repeats: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeldOutMetrics {
    pub repository_task_split: String,
    pub brier_score: f64,
    pub cohort_base_rate_brier: f64,
    pub brier_improvement_fraction: f64,
    pub expected_calibration_error: f64,
    pub calibration_slope: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FeatureScale {
    pub mean: f64,
    pub scale: f64,
}

#[derive(Debug, Clone)]
pub struct RuntimeFeatures {
    pub agent_variant: String,
    pub task_family: String,
    pub repository_id: String,
    pub language: String,
    pub freshness: String,
    pub environment_match: String,
    pub negative_control_status: String,
    pub verifier_independent: bool,
    pub verification_strength: f64,
    pub attempts: f64,
    pub flakiness: f64,
    pub change_size: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TrainerIdentity {
    pub python_version: String,
    pub pymc_version: String,
    pub numpy_version: String,
    pub seed: u64,
    pub single_threaded: bool,
    pub chains: u64,
    pub draws_per_chain: u64,
    pub tune_per_chain: u64,
    pub split_kind: String,
    pub toolchain_lock_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HierarchicalEligibility {
    pub eligible: bool,
    pub effective_outcomes: f64,
    pub exact_model_agent_variants: u64,
    pub task_families_with_50_outcomes: u64,
    pub required_effective_outcomes: u64,
    pub required_variants: u64,
    pub required_task_families: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CalibrationBundle {
    pub format_version: String,
    pub record_kind: String,
    pub methodology_version: String,
    pub model_kind: String,
    pub dataset_hash_alg: String,
    pub dataset_hash: String,
    pub source_dataset_hashes: Vec<String>,
    pub feature_dictionary: BTreeMap<String, String>,
    pub valid_domains: BTreeMap<String, Vec<String>>,
    pub sample_counts: SampleCounts,
    pub publication_state: String,
    pub publication_reason: String,
    pub posterior: PosteriorSummary,
    pub cohorts: Vec<CohortCalibration>,
    pub held_out_metrics: Option<HeldOutMetrics>,
    pub hierarchical_eligibility: HierarchicalEligibility,
    pub posterior_draws: Vec<f64>,
    pub model_parameters: BTreeMap<String, Vec<f64>>,
    pub feature_scaling: BTreeMap<String, FeatureScale>,
    pub trainer: Option<TrainerIdentity>,
    pub executor: ExecutorIdentity,
    pub engine: SignedEngineIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub bundle_hash: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationBuildReport {
    pub ok: bool,
    pub publication_state: String,
    pub effective_outcomes: f64,
    pub dataset_hash: String,
    pub bundle: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CalibrationObservation {
    pub source: String,
    pub record_hash: String,
    pub task_id: String,
    pub observation_key: String,
    pub result: String,
    pub repository_id: String,
    pub language: String,
    pub weight: f64,
    pub freshness: String,
    pub environment_match: String,
    pub verification_strength: f64,
    pub negative_control_status: String,
    pub verifier_independent: bool,
    pub attempts: u32,
    pub flakiness: f64,
    pub change_size: u64,
    pub split: String,
    pub cohort: CohortIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CalibrationDataset {
    pub format_version: String,
    pub record_kind: String,
    pub methodology_version: String,
    pub dataset_hash_alg: String,
    pub dataset_hash: String,
    pub split_kind: String,
    pub source_dataset_hashes: Vec<String>,
    pub observations: Vec<CalibrationObservation>,
    pub executor: ExecutorIdentity,
    pub engine: SignedEngineIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub record_hash: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrationDatasetReport {
    pub ok: bool,
    pub dataset_hash: String,
    pub eligible_outcomes: u64,
    pub dataset: PathBuf,
}

struct CollectedData {
    raw_outcomes: u64,
    eligible_outcomes: u64,
    source_dataset_hashes: Vec<String>,
    observations: Vec<CalibrationObservation>,
    grouped: BTreeMap<CohortIdentity, BTreeMap<String, Vec<(f64, f64)>>>,
}

fn observation_key(cohort: &CohortIdentity, task_id: &str) -> String {
    blake3::hash(
        &serde_json::to_vec(&serde_json::json!([
            cohort.provider,
            cohort.model_snapshot,
            cohort.agent_name,
            cohort.agent_version,
            cohort.task_family,
            task_id,
        ]))
        .expect("cohort identity is JSON serializable"),
    )
    .to_hex()
    .to_string()
}

fn discover_run_dirs(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    if !root.is_dir() {
        return Err(format!("calibration runs directory not found: {}", root.display()).into());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut discovered = Vec::new();
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        visited += 1;
        if visited > 10_000 {
            return Err("calibration run discovery exceeded 10,000 directories".into());
        }
        if directory.join("manifest.json").is_file()
            && directory.join("outcomes/outcomes.jsonl").is_file()
        {
            discovered.push(directory.clone());
        }
        let mut children = Vec::new();
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                children.push(entry.path());
            }
        }
        children.sort();
        pending.extend(children.into_iter().rev());
    }
    discovered.sort();
    Ok(discovered)
}

fn exact_cohort(
    outcome: &IndependentOutcome,
) -> Result<CohortIdentity, Box<dyn std::error::Error>> {
    let required = |label: &str, value: &Option<String>| {
        value
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .ok_or_else(|| format!("training-eligible outcome is missing exact {label}"))
    };
    Ok(CohortIdentity {
        provider: required("provider", &outcome.model.provider)?,
        model_snapshot: required(
            "resolved_model_snapshot",
            &outcome.model.resolved_model_snapshot,
        )?,
        agent_name: required("agent_name", &outcome.model.agent_name)?,
        agent_version: required("agent_version", &outcome.model.agent_version)?,
        task_family: if outcome.task_family.trim().is_empty() {
            return Err("training-eligible outcome is missing task_family".into());
        } else {
            outcome.task_family.clone()
        },
    })
}

fn ln_gamma(value: f64) -> f64 {
    const COEFFICIENTS: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if value < 0.5 {
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * value).sin().ln()
            - ln_gamma(1.0 - value);
    }
    let z = value - 1.0;
    let mut sum = COEFFICIENTS[0];
    for (index, coefficient) in COEFFICIENTS.iter().enumerate().skip(1) {
        sum += coefficient / (z + index as f64);
    }
    let t = z + 7.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (z + 0.5) * t.ln() - t + sum.ln()
}

fn beta_fraction(a: f64, b: f64, x: f64) -> f64 {
    const MAX_ITERATIONS: usize = 200;
    const EPSILON: f64 = 3.0e-14;
    const FLOOR: f64 = 1.0e-300;
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < FLOOR {
        d = FLOOR;
    }
    d = 1.0 / d;
    let mut result = d;
    for iteration in 1..=MAX_ITERATIONS {
        let m = iteration as f64;
        let m2 = 2.0 * m;
        let mut aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < FLOOR {
            d = FLOOR;
        }
        c = 1.0 + aa / c;
        if c.abs() < FLOOR {
            c = FLOOR;
        }
        d = 1.0 / d;
        result *= d * c;
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < FLOOR {
            d = FLOOR;
        }
        c = 1.0 + aa / c;
        if c.abs() < FLOOR {
            c = FLOOR;
        }
        d = 1.0 / d;
        let delta = d * c;
        result *= delta;
        if (delta - 1.0).abs() < EPSILON {
            break;
        }
    }
    result
}

fn beta_cdf(x: f64, alpha: f64, beta: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front = (ln_gamma(alpha + beta) - ln_gamma(alpha) - ln_gamma(beta)
        + alpha * x.ln()
        + beta * (1.0 - x).ln())
    .exp();
    if x < (alpha + 1.0) / (alpha + beta + 2.0) {
        front * beta_fraction(alpha, beta, x) / alpha
    } else {
        1.0 - front * beta_fraction(beta, alpha, 1.0 - x) / beta
    }
}

pub(crate) fn beta_quantile(probability: f64, alpha: f64, beta: f64) -> f64 {
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..100 {
        let middle = (low + high) / 2.0;
        if beta_cdf(middle, alpha, beta) < probability {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0
}

fn suppressed_posterior() -> PosteriorSummary {
    PosteriorSummary {
        prior_alpha: 1.0,
        prior_beta: 1.0,
        posterior_alpha: None,
        posterior_beta: None,
        false_green_probability: None,
        upper_95_false_green_risk: None,
        interval_95_width: None,
    }
}

fn cohort_posterior(effective: f64, failures: f64) -> PosteriorSummary {
    if effective < 30.0 {
        return suppressed_posterior();
    }
    let alpha = 1.0 + failures;
    let beta = 1.0 + effective - failures;
    PosteriorSummary {
        prior_alpha: 1.0,
        prior_beta: 1.0,
        posterior_alpha: Some(alpha),
        posterior_beta: Some(beta),
        false_green_probability: Some(alpha / (alpha + beta)),
        upper_95_false_green_risk: Some(beta_quantile(0.95, alpha, beta)),
        interval_95_width: Some(
            beta_quantile(0.975, alpha, beta) - beta_quantile(0.025, alpha, beta),
        ),
    }
}

fn feature_dictionary() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "agent_variant".into(),
            "exact agent name and version".into(),
        ),
        (
            "attempts".into(),
            "complete attempts-to-green history".into(),
        ),
        (
            "change_size".into(),
            "measured files, additions, and deletions".into(),
        ),
        (
            "check_strength".into(),
            "bound checks and negative controls".into(),
        ),
        (
            "environment_match".into(),
            "check-to-outcome environment match".into(),
        ),
        (
            "flakiness".into(),
            "pass/fail transitions across all attempts".into(),
        ),
        (
            "language".into(),
            "implementation language when known".into(),
        ),
        (
            "model_snapshot".into(),
            "provider-resolved immutable model snapshot".into(),
        ),
        (
            "repository_difficulty".into(),
            "held-out repository or item effect".into(),
        ),
        ("task_family".into(), "versioned task family".into()),
        (
            "verifier_independence".into(),
            "authenticated adjudicator separation".into(),
        ),
    ])
}

fn canonical_unsigned(bundle: &CalibrationBundle) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut unsigned = bundle.clone();
    unsigned.bundle_hash.clear();
    unsigned.signature.clear();
    Ok(serde_json::to_vec(&unsigned)?)
}

fn bundle_hash(bundle: &CalibrationBundle) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(BUNDLE_HASH_DOMAIN);
    hasher.update(&canonical_unsigned(bundle)?);
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn verify_bundle(bundle: &CalibrationBundle) -> Result<(), Box<dyn std::error::Error>> {
    let baseline = bundle.methodology_version == "beta-binomial-v1"
        && bundle.model_kind == "baseline"
        && bundle.trainer.is_none();
    let hierarchical = bundle.methodology_version == "hierarchical-logistic-v1"
        && bundle.model_kind == "hierarchical"
        && bundle.trainer.is_some();
    if bundle.format_version != "1"
        || bundle.record_kind != "calibration_bundle"
        || (!baseline && !hierarchical)
        || bundle.dataset_hash_alg != "blake3-256"
        || bundle.hash_alg != "blake3-256"
        || bundle.signature_alg != "ed25519"
        || bundle.dataset_hash.len() != 64
        || bundle.bundle_hash.len() != 64
        || !matches!(
            bundle.publication_state.as_str(),
            "insufficient_data" | "provisional" | "calibrated"
        )
    {
        return Err("calibration bundle has unsupported metadata".into());
    }
    if bundle.publication_state == "insufficient_data"
        && (bundle.posterior.false_green_probability.is_some()
            || bundle.posterior.upper_95_false_green_risk.is_some())
    {
        return Err("insufficient-data bundle must suppress headline probability".into());
    }
    if bundle.publication_state == "calibrated" {
        let metrics = bundle
            .held_out_metrics
            .as_ref()
            .ok_or("calibrated bundle is missing held-out metrics")?;
        if !hierarchical
            || !bundle.hierarchical_eligibility.eligible
            || metrics.brier_improvement_fraction < 0.05
            || metrics.expected_calibration_error > 0.05
            || !(0.8..=1.2).contains(&metrics.calibration_slope)
            || bundle
                .posterior
                .interval_95_width
                .is_none_or(|width| width > 0.20)
        {
            return Err("calibrated bundle does not satisfy fixed release gates".into());
        }
    }
    let actual = bundle_hash(bundle)?;
    if actual != bundle.bundle_hash {
        return Err("calibration bundle hash mismatch".into());
    }
    verify_detached(
        BUNDLE_SIGNATURE_DOMAIN,
        bundle.bundle_hash.as_bytes(),
        &bundle.executor,
        &bundle.signature,
    )
}

pub fn load_verified_bundle(path: &Path) -> Result<CalibrationBundle, Box<dyn std::error::Error>> {
    let bundle: CalibrationBundle = serde_json::from_slice(&fs::read(path)?)?;
    verify_bundle(&bundle)?;
    Ok(bundle)
}

fn collect_data(
    runs_root: &Path,
    imports_dir: Option<&Path>,
) -> Result<CollectedData, Box<dyn std::error::Error>> {
    let mut raw_outcomes = 0u64;
    let mut eligible_outcomes = 0u64;
    let mut observations = Vec::new();
    let mut grouped: BTreeMap<CohortIdentity, BTreeMap<String, Vec<(f64, f64)>>> = BTreeMap::new();
    let mut source_dataset_hashes = Vec::new();
    for run_dir in discover_run_dirs(runs_root)? {
        for (outcome, record_hash) in load_outcome_records(&run_dir)? {
            raw_outcomes += 1;
            if outcome.training_eligibility != "included" {
                continue;
            }
            let failure = match outcome.result.as_str() {
                "success" => 0.0,
                "failure" => 1.0,
                _ => return Err("included calibration outcome must be success or failure".into()),
            };
            let cohort = exact_cohort(&outcome)?;
            let attempts = outcome.retry_history.iter().map(|item| item.attempts).sum();
            let flakiness = if outcome.retry_history.is_empty() {
                0.0
            } else {
                outcome
                    .retry_history
                    .iter()
                    .map(|item| item.flake_rate)
                    .sum::<f64>()
                    / outcome.retry_history.len() as f64
            };
            let has_negative_control = !outcome
                .check_strength
                .contains("0 expected-failure negative control");
            let verification_strength =
                outcome.retry_history.len() as f64 + f64::from(has_negative_control);
            let change_size = u64::from(outcome.change_size.files)
                + u64::from(outcome.change_size.additions)
                + u64::from(outcome.change_size.deletions);
            eligible_outcomes += 1;
            observations.push(CalibrationObservation {
                source: "local_signed_outcome".to_string(),
                record_hash,
                task_id: outcome.task_id.clone(),
                observation_key: observation_key(&cohort, &outcome.task_id),
                result: outcome.result,
                repository_id: outcome.repository_id,
                language: outcome.language,
                weight: 1.0,
                freshness: outcome.freshness,
                environment_match: outcome.environment_match,
                verification_strength,
                negative_control_status: if has_negative_control {
                    "passed".to_string()
                } else {
                    "not_run".to_string()
                },
                verifier_independent: outcome.adjudicator.independent_from_worker,
                attempts,
                flakiness,
                change_size,
                split: String::new(),
                cohort: cohort.clone(),
            });
            grouped
                .entry(cohort)
                .or_default()
                .entry(outcome.task_id)
                .or_default()
                .push((failure, 1.0));
        }
    }
    if let Some(imports_dir) = imports_dir {
        let imported = load_verified_imports(imports_dir)?;
        source_dataset_hashes = imported.source_dataset_hashes;
        for result in imported.task_results {
            raw_outcomes += 1;
            eligible_outcomes += 1;
            let failure = if result.result == "failure" { 1.0 } else { 0.0 };
            let cohort = CohortIdentity {
                provider: result.provider,
                model_snapshot: result.model_snapshot,
                agent_name: result.agent_name,
                agent_version: result.agent_version,
                task_family: result.task_family,
            };
            observations.push(CalibrationObservation {
                source: "external_fractional_prior".to_string(),
                record_hash: format!("{}:{}", result.dataset_hash, result.task_id),
                task_id: result.task_id.clone(),
                observation_key: observation_key(&cohort, &result.task_id),
                result: result.result,
                repository_id: result.repository_id,
                language: result.language,
                weight: result.weight,
                freshness: "unknown".to_string(),
                environment_match: "unknown".to_string(),
                verification_strength: 0.0,
                negative_control_status: "unknown".to_string(),
                verifier_independent: true,
                attempts: 1,
                flakiness: 0.0,
                change_size: 0,
                split: String::new(),
                cohort: cohort.clone(),
            });
            grouped
                .entry(cohort)
                .or_default()
                .entry(result.task_id)
                .or_default()
                .push((failure, result.weight));
        }
    }
    observations.sort_by(|left, right| {
        (&left.record_hash, &left.task_id).cmp(&(&right.record_hash, &right.task_id))
    });
    assign_splits(&mut observations);
    Ok(CollectedData {
        raw_outcomes,
        eligible_outcomes,
        source_dataset_hashes,
        observations,
        grouped,
    })
}

fn selected_holdout_values(values: &BTreeSet<String>) -> BTreeSet<String> {
    let mut selected: BTreeSet<String> = values
        .iter()
        .filter(|value| blake3::hash(value.as_bytes()).as_bytes()[0] < 51)
        .cloned()
        .collect();
    if selected.is_empty() {
        if let Some(value) = values
            .iter()
            .min_by_key(|value| blake3::hash(value.as_bytes()).to_hex().to_string())
        {
            selected.insert(value.clone());
        }
    }
    if selected.len() == values.len() && values.len() > 1 {
        if let Some(value) = selected
            .iter()
            .max_by_key(|value| blake3::hash(value.as_bytes()).to_hex().to_string())
            .cloned()
        {
            selected.remove(&value);
        }
    }
    selected
}

fn assign_splits(observations: &mut [CalibrationObservation]) -> &'static str {
    let repositories: BTreeSet<String> = observations
        .iter()
        .map(|row| row.repository_id.clone())
        .collect();
    if repositories.len() >= 5 {
        let heldout = selected_holdout_values(&repositories);
        for row in observations {
            row.split = if heldout.contains(&row.repository_id) {
                "heldout".to_string()
            } else {
                "train".to_string()
            };
        }
        "held-out-repositories"
    } else {
        let tasks: BTreeSet<String> = observations.iter().map(|row| row.task_id.clone()).collect();
        let heldout = selected_holdout_values(&tasks);
        for row in observations {
            row.split = if heldout.contains(&row.task_id) {
                "heldout".to_string()
            } else {
                "train".to_string()
            };
        }
        "held-out-tasks"
    }
}

fn dataset_content_hash(
    sources: &[String],
    observations: &[CalibrationObservation],
) -> Result<String, Box<dyn std::error::Error>> {
    Ok(blake3::hash(&serde_json::to_vec(&serde_json::json!({
        "source_dataset_hashes": sources,
        "observations": observations,
    }))?)
    .to_hex()
    .to_string())
}

fn canonical_unsigned_dataset(
    dataset: &CalibrationDataset,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut unsigned = dataset.clone();
    unsigned.record_hash.clear();
    unsigned.signature.clear();
    Ok(serde_json::to_vec(&unsigned)?)
}

fn dataset_record_hash(dataset: &CalibrationDataset) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(DATASET_HASH_DOMAIN);
    hasher.update(&canonical_unsigned_dataset(dataset)?);
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn verify_dataset(dataset: &CalibrationDataset) -> Result<(), Box<dyn std::error::Error>> {
    if dataset.format_version != "1"
        || dataset.record_kind != "calibration_dataset"
        || dataset.methodology_version != "hierarchical-logistic-v1"
        || dataset.dataset_hash_alg != "blake3-256"
        || !matches!(
            dataset.split_kind.as_str(),
            "held-out-repositories" | "held-out-tasks"
        )
        || dataset.hash_alg != "blake3-256"
        || dataset.signature_alg != "ed25519"
        || dataset.dataset_hash.len() != 64
        || dataset.record_hash.len() != 64
    {
        return Err("calibration dataset has unsupported metadata".into());
    }
    if dataset
        .observations
        .iter()
        .any(|row| !matches!(row.split.as_str(), "train" | "heldout"))
    {
        return Err("calibration dataset contains an invalid split assignment".into());
    }
    let content_hash = dataset_content_hash(&dataset.source_dataset_hashes, &dataset.observations)?;
    if content_hash != dataset.dataset_hash {
        return Err("calibration dataset content hash mismatch".into());
    }
    let actual = dataset_record_hash(dataset)?;
    if actual != dataset.record_hash {
        return Err("calibration dataset record hash mismatch".into());
    }
    verify_detached(
        DATASET_SIGNATURE_DOMAIN,
        dataset.record_hash.as_bytes(),
        &dataset.executor,
        &dataset.signature,
    )
}

pub fn load_verified_dataset(
    path: &Path,
) -> Result<CalibrationDataset, Box<dyn std::error::Error>> {
    let dataset: CalibrationDataset = serde_json::from_slice(&fs::read(path)?)?;
    verify_dataset(&dataset)?;
    Ok(dataset)
}

pub fn export_dataset(
    runs_root: &Path,
    imports_dir: Option<&Path>,
    out: &Path,
) -> Result<CalibrationDatasetReport, Box<dyn std::error::Error>> {
    let collected = collect_data(runs_root, imports_dir)?;
    let split_kind = if collected
        .observations
        .iter()
        .map(|row| &row.repository_id)
        .collect::<BTreeSet<_>>()
        .len()
        >= 5
    {
        "held-out-repositories"
    } else {
        "held-out-tasks"
    };
    let dataset_hash =
        dataset_content_hash(&collected.source_dataset_hashes, &collected.observations)?;
    let mut dataset = CalibrationDataset {
        format_version: "1".to_string(),
        record_kind: "calibration_dataset".to_string(),
        methodology_version: "hierarchical-logistic-v1".to_string(),
        dataset_hash_alg: "blake3-256".to_string(),
        dataset_hash: dataset_hash.clone(),
        split_kind: split_kind.to_string(),
        source_dataset_hashes: collected.source_dataset_hashes,
        observations: collected.observations,
        executor: current_executor_identity()?,
        engine: current_engine_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        record_hash: String::new(),
        signature: String::new(),
    };
    dataset.record_hash = dataset_record_hash(&dataset)?;
    let (executor, signature) =
        sign_detached(DATASET_SIGNATURE_DOMAIN, dataset.record_hash.as_bytes())?;
    if executor != dataset.executor {
        return Err("executor identity changed while signing calibration dataset".into());
    }
    dataset.signature = signature;
    verify_dataset(&dataset)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(&dataset)?;
    bytes.push(b'\n');
    fs::write(out, bytes)?;
    Ok(CalibrationDatasetReport {
        ok: true,
        dataset_hash,
        eligible_outcomes: dataset.observations.len() as u64,
        dataset: out.to_path_buf(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainerMetrics {
    brier_score: f64,
    cohort_base_rate_brier: f64,
    brier_improvement_fraction: f64,
    expected_calibration_error: f64,
    calibration_slope: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainerPrediction {
    observation_key: String,
    actual_failure: f64,
    predicted_failure: f64,
    weight: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainerOutput {
    format_version: String,
    methodology_version: String,
    dataset_hash: String,
    seed: u64,
    single_threaded: bool,
    chains: u64,
    draws_per_chain: u64,
    tune_per_chain: u64,
    python_version: String,
    pymc_version: String,
    numpy_version: String,
    split_kind: String,
    training_observation_keys: Vec<String>,
    held_out_predictions: Vec<TrainerPrediction>,
    metrics: TrainerMetrics,
    posterior_draws: Vec<f64>,
    feature_scaling: BTreeMap<String, FeatureScale>,
    feature_domains: BTreeMap<String, Vec<String>>,
    model_parameters: BTreeMap<String, Vec<f64>>,
}

#[derive(Debug, Clone)]
struct ClusteredObservation {
    observation_key: String,
    actual_failure: f64,
    weight: f64,
    split: String,
    cohort: CohortIdentity,
    repository_id: String,
    language: String,
    freshness: String,
    environment_match: String,
    verification_strength: f64,
    negative_control_status: String,
    verifier_independent: bool,
    attempts: f64,
    flakiness: f64,
    change_size: f64,
}

fn cluster_observations(
    observations: &[CalibrationObservation],
) -> Result<Vec<ClusteredObservation>, Box<dyn std::error::Error>> {
    let mut grouped: BTreeMap<String, Vec<&CalibrationObservation>> = BTreeMap::new();
    for row in observations {
        if row.observation_key != observation_key(&row.cohort, &row.task_id) {
            return Err("calibration observation key does not bind exact cohort and task".into());
        }
        grouped
            .entry(row.observation_key.clone())
            .or_default()
            .push(row);
    }
    grouped
        .into_iter()
        .map(|(key, members)| {
            let first = members[0];
            if members.iter().any(|row| {
                row.cohort != first.cohort
                    || row.split != first.split
                    || row.repository_id != first.repository_id
                    || row.language != first.language
                    || row.freshness != first.freshness
                    || row.environment_match != first.environment_match
                    || row.negative_control_status != first.negative_control_status
                    || row.verifier_independent != first.verifier_independent
            }) {
                return Err(
                    "repeated task crosses cohort, feature domain, or held-out split".into(),
                );
            }
            let total_weight: f64 = members.iter().map(|row| row.weight).sum();
            let cluster_weight = members.iter().map(|row| row.weight).fold(0.0, f64::max);
            if total_weight <= 0.0 || cluster_weight > 1.0 {
                return Err("calibration observation weight is outside (0,1]".into());
            }
            let failures: f64 = members
                .iter()
                .map(|row| {
                    if row.result == "failure" {
                        row.weight
                    } else {
                        0.0
                    }
                })
                .sum();
            Ok(ClusteredObservation {
                observation_key: key,
                actual_failure: failures / total_weight,
                weight: cluster_weight,
                split: first.split.clone(),
                cohort: first.cohort.clone(),
                repository_id: first.repository_id.clone(),
                language: first.language.clone(),
                freshness: first.freshness.clone(),
                environment_match: first.environment_match.clone(),
                verification_strength: members
                    .iter()
                    .map(|row| row.verification_strength)
                    .sum::<f64>()
                    / members.len() as f64,
                negative_control_status: first.negative_control_status.clone(),
                verifier_independent: first.verifier_independent,
                attempts: members
                    .iter()
                    .map(|row| f64::from(row.attempts))
                    .sum::<f64>()
                    / members.len() as f64,
                flakiness: members.iter().map(|row| row.flakiness).sum::<f64>()
                    / members.len() as f64,
                change_size: members
                    .iter()
                    .map(|row| row.change_size as f64)
                    .sum::<f64>()
                    / members.len() as f64,
            })
        })
        .collect()
}

fn category_value(row: &ClusteredObservation, feature: &str) -> Option<String> {
    match feature {
        "agent_variant" => Some(format!(
            "{}:{}:{}:{}",
            row.cohort.provider,
            row.cohort.model_snapshot,
            row.cohort.agent_name,
            row.cohort.agent_version
        )),
        "task_family" => Some(row.cohort.task_family.clone()),
        "repository_id" => Some(row.repository_id.clone()),
        "language" => Some(row.language.clone()),
        "freshness" => Some(row.freshness.clone()),
        "environment_match" => Some(row.environment_match.clone()),
        "negative_control_status" => Some(row.negative_control_status.clone()),
        "verifier_independent" => Some(row.verifier_independent.to_string()),
        _ => None,
    }
}

fn model_predictions(
    rows: &[ClusteredObservation],
    parameters: &BTreeMap<String, Vec<f64>>,
    scaling: &BTreeMap<String, FeatureScale>,
) -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    let intercept = parameters
        .get("intercept")
        .ok_or("trainer model is missing intercept")?;
    let numeric: [(&str, fn(&ClusteredObservation) -> f64); 4] = [
        ("verification_strength", |row: &ClusteredObservation| {
            row.verification_strength
        }),
        ("attempts", |row: &ClusteredObservation| row.attempts),
        ("flakiness", |row: &ClusteredObservation| row.flakiness),
        ("change_size", |row: &ClusteredObservation| row.change_size),
    ];
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        let mut sum = 0.0;
        for draw in 0..intercept.len() {
            let mut logit = intercept[draw];
            for (feature, value) in numeric {
                let scale = scaling
                    .get(feature)
                    .ok_or_else(|| format!("trainer model is missing scale `{feature}`"))?;
                let beta = parameters
                    .get(&format!("numeric:{feature}"))
                    .ok_or_else(|| format!("trainer model is missing numeric `{feature}`"))?;
                logit += beta[draw] * ((value(row) - scale.mean) / scale.scale);
            }
            for feature in [
                "agent_variant",
                "task_family",
                "repository_id",
                "language",
                "freshness",
                "environment_match",
                "negative_control_status",
                "verifier_independent",
            ] {
                let value = category_value(row, feature).expect("known categorical feature");
                if let Some(effect) = parameters.get(&format!("{feature}:{value}")) {
                    logit += effect[draw];
                }
            }
            sum += 1.0 / (1.0 + (-logit.clamp(-30.0, 30.0)).exp());
        }
        results.push(sum / intercept.len() as f64);
    }
    Ok(results)
}

fn hierarchical_eligibility(clusters: &[ClusteredObservation]) -> HierarchicalEligibility {
    let effective: f64 = clusters.iter().map(|row| row.weight).sum();
    let variants: BTreeSet<String> = clusters
        .iter()
        .map(|row| {
            format!(
                "{}:{}:{}:{}",
                row.cohort.provider,
                row.cohort.model_snapshot,
                row.cohort.agent_name,
                row.cohort.agent_version
            )
        })
        .collect();
    let mut families: BTreeMap<String, f64> = BTreeMap::new();
    for row in clusters {
        *families.entry(row.cohort.task_family.clone()).or_default() += row.weight;
    }
    let eligible_families = families.values().filter(|count| **count >= 50.0).count() as u64;
    HierarchicalEligibility {
        eligible: effective >= 500.0 && variants.len() >= 5 && eligible_families >= 3,
        effective_outcomes: effective,
        exact_model_agent_variants: variants.len() as u64,
        task_families_with_50_outcomes: eligible_families,
        required_effective_outcomes: 500,
        required_variants: 5,
        required_task_families: 3,
    }
}

fn calibration_slope(actual: &[f64], predicted: &[f64], weights: &[f64]) -> f64 {
    let mut intercept = 0.0;
    let mut slope = 1.0;
    for _ in 0..50 {
        let mut gradient = [0.0, 0.0];
        let mut hessian = [[0.0, 0.0], [0.0, 0.0]];
        for ((actual, predicted), weight) in actual.iter().zip(predicted).zip(weights) {
            let probability = predicted.clamp(1.0e-6, 1.0 - 1.0e-6);
            let logit = (probability / (1.0 - probability)).ln();
            let fitted = 1.0 / (1.0 + (-(intercept + slope * logit)).exp());
            let residual = weight * (actual - fitted);
            let variance = weight * fitted * (1.0 - fitted);
            gradient[0] += residual;
            gradient[1] += residual * logit;
            hessian[0][0] -= variance;
            hessian[0][1] -= variance * logit;
            hessian[1][0] -= variance * logit;
            hessian[1][1] -= variance * logit * logit;
        }
        let determinant = hessian[0][0] * hessian[1][1] - hessian[0][1] * hessian[1][0];
        if determinant.abs() < 1.0e-12 {
            return 0.0;
        }
        let step_intercept =
            (hessian[1][1] * gradient[0] - hessian[0][1] * gradient[1]) / determinant;
        let step_slope = (-hessian[1][0] * gradient[0] + hessian[0][0] * gradient[1]) / determinant;
        intercept -= step_intercept;
        slope -= step_slope;
        if step_intercept.abs().max(step_slope.abs()) < 1.0e-8 {
            break;
        }
    }
    slope
}

fn recompute_metrics(
    training: &[ClusteredObservation],
    heldout: &[ClusteredObservation],
    predictions: &[f64],
) -> Result<HeldOutMetrics, Box<dyn std::error::Error>> {
    if heldout.is_empty() || heldout.len() != predictions.len() {
        return Err("held-out prediction coverage is empty or incomplete".into());
    }
    let total_weight: f64 = heldout.iter().map(|row| row.weight).sum();
    let mut train_rates: BTreeMap<CohortIdentity, (f64, f64)> = BTreeMap::new();
    for row in training {
        let entry = train_rates.entry(row.cohort.clone()).or_default();
        entry.0 += row.actual_failure * row.weight;
        entry.1 += row.weight;
    }
    let global_failures: f64 = training
        .iter()
        .map(|row| row.actual_failure * row.weight)
        .sum();
    let global_weight: f64 = training.iter().map(|row| row.weight).sum();
    if total_weight <= 0.0 || global_weight <= 0.0 {
        return Err("calibration split has zero effective training or held-out weight".into());
    }
    let global_rate = global_failures / global_weight;
    let mut brier = 0.0;
    let mut base_brier = 0.0;
    let mut actual = Vec::new();
    let mut weights = Vec::new();
    for (row, prediction) in heldout.iter().zip(predictions) {
        if !prediction.is_finite() || !(0.0..=1.0).contains(prediction) {
            return Err("held-out prediction is not a finite probability".into());
        }
        let base_rate = train_rates
            .get(&row.cohort)
            .map(|(failures, weight)| failures / weight)
            .unwrap_or(global_rate);
        brier += row.weight * (prediction - row.actual_failure).powi(2);
        base_brier += row.weight * (base_rate - row.actual_failure).powi(2);
        actual.push(row.actual_failure);
        weights.push(row.weight);
    }
    brier /= total_weight;
    base_brier /= total_weight;
    let improvement = if base_brier > 0.0 {
        (base_brier - brier) / base_brier
    } else {
        0.0
    };
    let mut ece = 0.0;
    for bin in 0..10 {
        let lower = bin as f64 / 10.0;
        let upper = (bin + 1) as f64 / 10.0;
        let mut bin_weight = 0.0;
        let mut observed = 0.0;
        let mut forecast = 0.0;
        for ((row, prediction), weight) in heldout.iter().zip(predictions).zip(&weights) {
            if *prediction >= lower && (*prediction < upper || (bin == 9 && *prediction <= 1.0)) {
                bin_weight += weight;
                observed += weight * row.actual_failure;
                forecast += weight * prediction;
            }
        }
        if bin_weight > 0.0 {
            ece +=
                bin_weight / total_weight * (observed / bin_weight - forecast / bin_weight).abs();
        }
    }
    Ok(HeldOutMetrics {
        repository_task_split: "deterministic BLAKE3 grouped holdout".to_string(),
        brier_score: brier,
        cohort_base_rate_brier: base_brier,
        brier_improvement_fraction: improvement,
        expected_calibration_error: ece,
        calibration_slope: calibration_slope(&actual, predictions, &weights),
    })
}

pub(crate) fn percentile(sorted: &[f64], probability: f64) -> f64 {
    let index = probability * (sorted.len() - 1) as f64;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        sorted[lower] + (sorted[upper] - sorted[lower]) * (index - lower as f64)
    }
}

pub(crate) fn runtime_hierarchical_draws(
    bundle: &CalibrationBundle,
    features: &RuntimeFeatures,
) -> Result<Vec<f64>, Box<dyn std::error::Error>> {
    if bundle.model_kind != "hierarchical" {
        return Err("runtime hierarchical scoring requires a hierarchical bundle".into());
    }
    let intercept = bundle
        .model_parameters
        .get("intercept")
        .ok_or("hierarchical bundle is missing intercept draws")?;
    let numeric = [
        ("verification_strength", features.verification_strength),
        ("attempts", features.attempts),
        ("flakiness", features.flakiness),
        ("change_size", features.change_size),
    ];
    let categorical = [
        ("agent_variant", features.agent_variant.clone()),
        ("task_family", features.task_family.clone()),
        ("repository_id", features.repository_id.clone()),
        ("language", features.language.clone()),
        ("freshness", features.freshness.clone()),
        ("environment_match", features.environment_match.clone()),
        (
            "negative_control_status",
            features.negative_control_status.clone(),
        ),
        (
            "verifier_independent",
            features.verifier_independent.to_string(),
        ),
    ];
    let mut draws = Vec::with_capacity(intercept.len());
    for draw in 0..intercept.len() {
        let mut logit = intercept[draw];
        for (name, value) in numeric {
            let scale = bundle
                .feature_scaling
                .get(name)
                .ok_or_else(|| format!("hierarchical bundle is missing `{name}` scale"))?;
            let beta = bundle
                .model_parameters
                .get(&format!("numeric:{name}"))
                .ok_or_else(|| format!("hierarchical bundle is missing `{name}` draws"))?;
            if beta.len() != intercept.len() {
                return Err("hierarchical numeric draw count mismatch".into());
            }
            logit += beta[draw] * ((value - scale.mean) / scale.scale);
        }
        for (name, value) in &categorical {
            if let Some(effect) = bundle.model_parameters.get(&format!("{name}:{value}")) {
                if effect.len() != intercept.len() {
                    return Err("hierarchical category draw count mismatch".into());
                }
                logit += effect[draw];
            }
        }
        draws.push(1.0 / (1.0 + (-logit.clamp(-30.0, 30.0)).exp()));
    }
    Ok(draws)
}

pub fn promote_hierarchical(
    dataset_path: &Path,
    trainer_output_path: &Path,
    toolchain_lock: &Path,
    out: &Path,
) -> Result<CalibrationBuildReport, Box<dyn std::error::Error>> {
    let dataset = load_verified_dataset(dataset_path)?;
    let trainer: TrainerOutput = serde_json::from_slice(&fs::read(trainer_output_path)?)?;
    if trainer.format_version != "1"
        || trainer.methodology_version != "hierarchical-logistic-v1"
        || trainer.dataset_hash != dataset.dataset_hash
        || trainer.seed != 20260714
        || !trainer.single_threaded
        || trainer.chains != 2
        || trainer.draws_per_chain < 1000
        || trainer.tune_per_chain < 1000
        || !trainer.python_version.starts_with("3.12.")
        || trainer.pymc_version.is_empty()
        || trainer.numpy_version.is_empty()
        || trainer.split_kind != dataset.split_kind
    {
        return Err("trainer output does not match the pinned hierarchical methodology".into());
    }
    let expected_draws = (trainer.chains * trainer.draws_per_chain) as usize;
    if trainer.posterior_draws.len() != expected_draws
        || trainer
            .posterior_draws
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        || trainer.model_parameters.values().any(|draws| {
            draws.len() != expected_draws || draws.iter().any(|value| !value.is_finite())
        })
        || trainer
            .feature_scaling
            .values()
            .any(|scale| !scale.mean.is_finite() || !scale.scale.is_finite() || scale.scale <= 0.0)
    {
        return Err("trainer posterior shape is incomplete or non-finite".into());
    }
    for required in [
        "intercept",
        "numeric:verification_strength",
        "numeric:attempts",
        "numeric:flakiness",
        "numeric:change_size",
    ] {
        if !trainer.model_parameters.contains_key(required) {
            return Err(format!("trainer output is missing model parameter `{required}`").into());
        }
    }
    for feature in [
        "verification_strength",
        "attempts",
        "flakiness",
        "change_size",
    ] {
        if !trainer.feature_scaling.contains_key(feature) {
            return Err(format!("trainer output is missing feature scale `{feature}`").into());
        }
    }
    for (feature, domain) in &trainer.feature_domains {
        for value in domain {
            let parameter = format!("{feature}:{value}");
            if !trainer.model_parameters.contains_key(&parameter) {
                return Err(
                    format!("trainer output is missing category parameter `{parameter}`").into(),
                );
            }
        }
    }
    let clusters = cluster_observations(&dataset.observations)?;
    let eligibility = hierarchical_eligibility(&clusters);
    if !eligibility.eligible {
        return Err(format!(
            "hierarchical release ineligible: effective={}, variants={}, families_50={}",
            eligibility.effective_outcomes,
            eligibility.exact_model_agent_variants,
            eligibility.task_families_with_50_outcomes
        )
        .into());
    }
    let training: Vec<_> = clusters
        .iter()
        .filter(|row| row.split == "train")
        .cloned()
        .collect();
    let heldout: Vec<_> = clusters
        .iter()
        .filter(|row| row.split == "heldout")
        .cloned()
        .collect();
    let expected_training: BTreeSet<_> = training
        .iter()
        .map(|row| row.observation_key.clone())
        .collect();
    let declared_training: BTreeSet<_> =
        trainer.training_observation_keys.iter().cloned().collect();
    if expected_training != declared_training
        || declared_training.len() != trainer.training_observation_keys.len()
    {
        return Err("trainer output training split does not match signed dataset".into());
    }
    let expected_heldout: BTreeMap<_, _> = heldout
        .iter()
        .map(|row| (row.observation_key.clone(), row))
        .collect();
    let mut predictions = BTreeMap::new();
    for prediction in &trainer.held_out_predictions {
        let expected = expected_heldout
            .get(&prediction.observation_key)
            .ok_or("trainer output contains an unexpected held-out observation")?;
        if (prediction.actual_failure - expected.actual_failure).abs() > 1.0e-9
            || (prediction.weight - expected.weight).abs() > 1.0e-9
            || predictions
                .insert(
                    prediction.observation_key.clone(),
                    prediction.predicted_failure,
                )
                .is_some()
        {
            return Err("trainer held-out target/weight is inconsistent or duplicated".into());
        }
    }
    if predictions.len() != expected_heldout.len() {
        return Err("trainer output omitted held-out observations".into());
    }
    let ordered_predictions: Vec<f64> = heldout
        .iter()
        .map(|row| predictions[&row.observation_key])
        .collect();
    let model_predictions = model_predictions(
        &heldout,
        &trainer.model_parameters,
        &trainer.feature_scaling,
    )?;
    if ordered_predictions
        .iter()
        .zip(&model_predictions)
        .any(|(declared, computed)| (declared - computed).abs() > 1.0e-6)
    {
        return Err("trainer held-out predictions disagree with exported runtime model".into());
    }
    let metrics = recompute_metrics(&training, &heldout, &ordered_predictions)?;
    for (claimed, actual) in [
        (trainer.metrics.brier_score, metrics.brier_score),
        (
            trainer.metrics.cohort_base_rate_brier,
            metrics.cohort_base_rate_brier,
        ),
        (
            trainer.metrics.brier_improvement_fraction,
            metrics.brier_improvement_fraction,
        ),
        (
            trainer.metrics.expected_calibration_error,
            metrics.expected_calibration_error,
        ),
        (trainer.metrics.calibration_slope, metrics.calibration_slope),
    ] {
        if !claimed.is_finite() || (claimed - actual).abs() > 1.0e-5 {
            return Err("trainer claimed metrics disagree with Rust recomputation".into());
        }
    }
    if metrics.brier_improvement_fraction < 0.05
        || metrics.expected_calibration_error > 0.05
        || !(0.8..=1.2).contains(&metrics.calibration_slope)
    {
        return Err(format!(
            "held-out calibration release gate failed: improvement={:.6}, ece={:.6}, slope={:.6}",
            metrics.brier_improvement_fraction,
            metrics.expected_calibration_error,
            metrics.calibration_slope
        )
        .into());
    }
    let mut draws = trainer.posterior_draws.clone();
    draws.sort_by(f64::total_cmp);
    let interval_width = percentile(&draws, 0.975) - percentile(&draws, 0.025);
    if interval_width > 0.20 {
        return Err(
            format!("posterior interval release gate failed: width={interval_width:.6}").into(),
        );
    }
    let posterior = PosteriorSummary {
        prior_alpha: 1.0,
        prior_beta: 1.0,
        posterior_alpha: None,
        posterior_beta: None,
        false_green_probability: Some(draws.iter().sum::<f64>() / draws.len() as f64),
        upper_95_false_green_risk: Some(percentile(&draws, 0.95)),
        interval_95_width: Some(interval_width),
    };
    let mut cohort_groups: BTreeMap<CohortIdentity, Vec<&ClusteredObservation>> = BTreeMap::new();
    for row in &clusters {
        cohort_groups
            .entry(row.cohort.clone())
            .or_default()
            .push(row);
    }
    let cohorts: Vec<CohortCalibration> = cohort_groups
        .into_iter()
        .map(|(cohort, rows)| {
            let effective: f64 = rows.iter().map(|row| row.weight).sum();
            let failures: f64 = rows.iter().map(|row| row.actual_failure * row.weight).sum();
            CohortCalibration {
                cohort,
                raw_outcomes: rows.len() as u64,
                effective_outcomes: effective,
                false_green_events: failures,
                publication_state: if effective < 30.0 {
                    "insufficient_data".to_string()
                } else {
                    "calibrated".to_string()
                },
                calibration_status: "hierarchical-held-out-passed".to_string(),
                posterior: cohort_posterior(effective, failures),
            }
        })
        .collect();
    let lock_digest = blake3::hash(&fs::read(toolchain_lock)?)
        .to_hex()
        .to_string();
    let trainer_identity = TrainerIdentity {
        python_version: trainer.python_version,
        pymc_version: trainer.pymc_version,
        numpy_version: trainer.numpy_version,
        seed: trainer.seed,
        single_threaded: trainer.single_threaded,
        chains: trainer.chains,
        draws_per_chain: trainer.draws_per_chain,
        tune_per_chain: trainer.tune_per_chain,
        split_kind: trainer.split_kind,
        toolchain_lock_digest: lock_digest,
    };
    let mut bundle = CalibrationBundle {
        format_version: "1".to_string(),
        record_kind: "calibration_bundle".to_string(),
        methodology_version: "hierarchical-logistic-v1".to_string(),
        model_kind: "hierarchical".to_string(),
        dataset_hash_alg: "blake3-256".to_string(),
        dataset_hash: dataset.dataset_hash.clone(),
        source_dataset_hashes: dataset.source_dataset_hashes,
        feature_dictionary: feature_dictionary(),
        valid_domains: trainer.feature_domains,
        sample_counts: SampleCounts {
            raw_outcomes: dataset.observations.len() as u64,
            eligible_outcomes: dataset.observations.len() as u64,
            effective_outcomes: eligibility.effective_outcomes,
            clustered_repeats: dataset.observations.len().saturating_sub(clusters.len()) as u64,
        },
        publication_state: "calibrated".to_string(),
        publication_reason: "hierarchical eligibility and held-out calibration gates passed"
            .to_string(),
        posterior,
        cohorts,
        held_out_metrics: Some(metrics),
        hierarchical_eligibility: eligibility,
        posterior_draws: trainer.posterior_draws,
        model_parameters: trainer.model_parameters,
        feature_scaling: trainer.feature_scaling,
        trainer: Some(trainer_identity),
        executor: current_executor_identity()?,
        engine: current_engine_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        bundle_hash: String::new(),
        signature: String::new(),
    };
    bundle.bundle_hash = bundle_hash(&bundle)?;
    let (executor, signature) =
        sign_detached(BUNDLE_SIGNATURE_DOMAIN, bundle.bundle_hash.as_bytes())?;
    if executor != bundle.executor {
        return Err("executor identity changed while signing hierarchical bundle".into());
    }
    bundle.signature = signature;
    verify_bundle(&bundle)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(&bundle)?;
    bytes.push(b'\n');
    fs::write(out, bytes)?;
    Ok(CalibrationBuildReport {
        ok: true,
        publication_state: "calibrated".to_string(),
        effective_outcomes: bundle.sample_counts.effective_outcomes,
        dataset_hash: bundle.dataset_hash.clone(),
        bundle: out.to_path_buf(),
    })
}

pub fn build_baseline(
    runs_root: &Path,
    imports_dir: Option<&Path>,
    out: &Path,
) -> Result<CalibrationBuildReport, Box<dyn std::error::Error>> {
    let collected = collect_data(runs_root, imports_dir)?;
    let raw_outcomes = collected.raw_outcomes;
    let eligible_outcomes = collected.eligible_outcomes;
    let source_dataset_hashes = collected.source_dataset_hashes;
    let dataset_hash = dataset_content_hash(&source_dataset_hashes, &collected.observations)?;
    let mut cohorts = Vec::new();
    let mut effective_total = 0.0;
    let mut variant_keys = BTreeSet::new();
    let mut family_effective: BTreeMap<String, f64> = BTreeMap::new();
    let mut domains: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut task_clusters = 0u64;
    for (cohort, tasks) in collected.grouped {
        task_clusters += tasks.len() as u64;
        let effective: f64 = tasks
            .values()
            .map(|attempts| {
                attempts
                    .iter()
                    .map(|(_, weight)| *weight)
                    .fold(0.0, f64::max)
            })
            .sum();
        let failures: f64 = tasks
            .values()
            .map(|attempts| {
                let total_weight: f64 = attempts.iter().map(|(_, weight)| *weight).sum();
                let weighted_failures: f64 = attempts
                    .iter()
                    .map(|(failure, weight)| failure * weight)
                    .sum();
                let cluster_weight = attempts
                    .iter()
                    .map(|(_, weight)| *weight)
                    .fold(0.0, f64::max);
                weighted_failures / total_weight * cluster_weight
            })
            .sum();
        effective_total += effective;
        variant_keys.insert(format!(
            "{}:{}:{}:{}",
            cohort.provider, cohort.model_snapshot, cohort.agent_name, cohort.agent_version
        ));
        *family_effective
            .entry(cohort.task_family.clone())
            .or_default() += effective;
        for (name, value) in [
            ("provider", cohort.provider.clone()),
            ("model_snapshot", cohort.model_snapshot.clone()),
            ("agent_name", cohort.agent_name.clone()),
            ("agent_version", cohort.agent_version.clone()),
            ("task_family", cohort.task_family.clone()),
        ] {
            domains.entry(name.to_string()).or_default().insert(value);
        }
        let posterior = cohort_posterior(effective, failures);
        cohorts.push(CohortCalibration {
            cohort,
            raw_outcomes: tasks.values().map(|items| items.len() as u64).sum(),
            effective_outcomes: effective,
            false_green_events: failures,
            publication_state: if effective < 30.0 {
                "insufficient_data".to_string()
            } else {
                "provisional".to_string()
            },
            calibration_status: if effective < 30.0 {
                "not_evaluated".to_string()
            } else {
                "held_out_metrics_unavailable".to_string()
            },
            posterior,
        });
    }
    let eligible_families = family_effective
        .values()
        .filter(|count| **count >= 50.0)
        .count() as u64;
    let hierarchical = HierarchicalEligibility {
        eligible: effective_total >= 500.0 && variant_keys.len() >= 5 && eligible_families >= 3,
        effective_outcomes: effective_total,
        exact_model_agent_variants: variant_keys.len() as u64,
        task_families_with_50_outcomes: eligible_families,
        required_effective_outcomes: 500,
        required_variants: 5,
        required_task_families: 3,
    };
    let publication_state = if cohorts
        .iter()
        .any(|cohort| cohort.publication_state == "provisional")
    {
        "provisional"
    } else {
        "insufficient_data"
    };
    let posterior = if cohorts.len() == 1 {
        cohorts[0].posterior.clone()
    } else {
        suppressed_posterior()
    };
    let mut bundle = CalibrationBundle {
        format_version: "1".to_string(),
        record_kind: "calibration_bundle".to_string(),
        methodology_version: "beta-binomial-v1".to_string(),
        model_kind: "baseline".to_string(),
        dataset_hash_alg: "blake3-256".to_string(),
        dataset_hash: dataset_hash.clone(),
        source_dataset_hashes,
        feature_dictionary: feature_dictionary(),
        valid_domains: domains
            .into_iter()
            .map(|(name, values)| (name, values.into_iter().collect()))
            .collect(),
        sample_counts: SampleCounts {
            raw_outcomes,
            eligible_outcomes,
            effective_outcomes: effective_total,
            clustered_repeats: eligible_outcomes.saturating_sub(task_clusters),
        },
        publication_state: publication_state.to_string(),
        publication_reason: if publication_state == "insufficient_data" {
            "fewer than 30 effective outcomes in every exact cohort".to_string()
        } else {
            "baseline posterior is provisional until held-out calibration passes".to_string()
        },
        posterior,
        cohorts,
        held_out_metrics: None,
        hierarchical_eligibility: hierarchical,
        posterior_draws: Vec::new(),
        model_parameters: BTreeMap::new(),
        feature_scaling: BTreeMap::new(),
        trainer: None,
        executor: current_executor_identity()?,
        engine: current_engine_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        bundle_hash: String::new(),
        signature: String::new(),
    };
    bundle.bundle_hash = bundle_hash(&bundle)?;
    let (executor, signature) =
        sign_detached(BUNDLE_SIGNATURE_DOMAIN, bundle.bundle_hash.as_bytes())?;
    if executor != bundle.executor {
        return Err("executor identity changed while signing calibration bundle".into());
    }
    bundle.signature = signature;
    verify_bundle(&bundle)?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut bytes = serde_json::to_vec_pretty(&bundle)?;
    bytes.push(b'\n');
    fs::write(out, bytes)?;
    Ok(CalibrationBuildReport {
        ok: true,
        publication_state: publication_state.to_string(),
        effective_outcomes: effective_total,
        dataset_hash,
        bundle: out.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beta_one_thirty_one_has_expected_upper_bound() {
        let upper = beta_quantile(0.95, 1.0, 31.0);
        assert!((upper - 0.0921).abs() < 0.001, "upper={upper}");
    }

    #[test]
    fn probability_is_suppressed_until_thirty_effective_outcomes() {
        assert!(
            cohort_posterior(29.0, 0.0)
                .false_green_probability
                .is_none()
        );
        let eligible = cohort_posterior(30.0, 0.0);
        assert!(eligible.false_green_probability.is_some());
        assert!(eligible.upper_95_false_green_risk.is_some());
    }

    fn cluster(index: usize, variants: usize, families: usize) -> ClusteredObservation {
        ClusteredObservation {
            observation_key: format!("key-{index}"),
            actual_failure: if index % 2 == 0 { 0.0 } else { 1.0 },
            weight: 1.0,
            split: if index % 5 == 0 { "heldout" } else { "train" }.to_string(),
            cohort: CohortIdentity {
                provider: "provider".to_string(),
                model_snapshot: format!("model-{}", index % variants),
                agent_name: "agent".to_string(),
                agent_version: format!("v{}", index % variants),
                task_family: format!("family-{}", index % families),
            },
            repository_id: format!("repo-{}", index % 10),
            language: "rust".to_string(),
            freshness: "current".to_string(),
            environment_match: "matched".to_string(),
            verification_strength: 1.0,
            negative_control_status: "passed".to_string(),
            verifier_independent: true,
            attempts: 1.0,
            flakiness: 0.0,
            change_size: 1.0,
        }
    }

    #[test]
    fn hierarchical_upgrade_requires_all_three_sample_gates() {
        let eligible: Vec<_> = (0..500).map(|index| cluster(index, 5, 3)).collect();
        assert!(hierarchical_eligibility(&eligible).eligible);
        assert!(!hierarchical_eligibility(&eligible[..499]).eligible);
        let four_variants: Vec<_> = (0..500).map(|index| cluster(index, 4, 3)).collect();
        assert!(!hierarchical_eligibility(&four_variants).eligible);
        let two_families: Vec<_> = (0..500).map(|index| cluster(index, 5, 2)).collect();
        assert!(!hierarchical_eligibility(&two_families).eligible);
    }

    #[test]
    fn uninformative_predictions_fail_fixed_calibration_gates() {
        let rows: Vec<_> = (0..40).map(|index| cluster(index, 1, 1)).collect();
        let training = &rows[..20];
        let heldout = &rows[20..];
        let predictions = vec![0.5; heldout.len()];
        let metrics = recompute_metrics(training, heldout, &predictions).unwrap();
        assert!(metrics.brier_improvement_fraction < 0.05);
        assert!(!(0.8..=1.2).contains(&metrics.calibration_slope));
    }

    #[test]
    fn calibrated_predictions_can_pass_fixed_release_metrics() {
        let training: Vec<_> = (0..100).map(|index| cluster(index, 1, 1)).collect();
        let mut heldout = Vec::new();
        let mut predictions = Vec::new();
        for index in 0..100 {
            let mut row = cluster(1_000 + index, 1, 1);
            row.actual_failure = if index < 10 { 1.0 } else { 0.0 };
            row.split = "heldout".to_string();
            heldout.push(row);
            predictions.push(0.1);
        }
        for index in 0..100 {
            let mut row = cluster(2_000 + index, 1, 1);
            row.actual_failure = if index < 90 { 1.0 } else { 0.0 };
            row.split = "heldout".to_string();
            heldout.push(row);
            predictions.push(0.9);
        }
        let metrics = recompute_metrics(&training, &heldout, &predictions).unwrap();
        assert!(metrics.brier_improvement_fraction >= 0.05);
        assert!(metrics.expected_calibration_error <= 0.05);
        assert!((0.8..=1.2).contains(&metrics.calibration_slope));
    }
}
