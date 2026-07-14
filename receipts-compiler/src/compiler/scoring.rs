//! Runtime reliability scoring. Categorical trust failures always suppress
//! statistical output; a probability can never turn a red task green.

use crate::compiler::calibration::{
    CalibrationBundle, RuntimeFeatures, beta_quantile, load_verified_bundle, percentile,
    runtime_hierarchical_draws,
};
use crate::compiler::checks::load_verified_attempts;
use crate::compiler::outcomes::{freshness, language, load_outcome_records, repository_id};
use crate::compiler::receipts::load_verified_receipts;
use crate::compiler::run_dir::{RunManifest, compile_run_dir};
use crate::compiler::session::latest_session;
use crate::schema::{EvidenceCoverage, NextPassPacket};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompletionInterval {
    pub rate: f64,
    pub lower_95: f64,
    pub upper_95: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScoreVersions {
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model_snapshot: Option<String>,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub scaffold_name: Option<String>,
    pub scaffold_version: Option<String>,
    pub engine_version: String,
    pub engine_build_commit: String,
    pub engine_binary_digest: String,
    pub check_versions: Vec<String>,
    pub harness_versions: Vec<String>,
    pub judge_identities: Vec<String>,
    pub dataset_hash: String,
    pub calibration_bundle_hash: String,
    pub methodology_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReliabilityScore {
    pub format_version: String,
    pub run_id: String,
    pub objective_id: String,
    pub score_status: String,
    pub calibration_status: String,
    pub task_family: String,
    pub false_green_probability: Option<f64>,
    pub upper_95_false_green_risk: Option<f64>,
    pub false_green_interval_95_width: Option<f64>,
    pub verified_completion: Option<CompletionInterval>,
    pub held_out_calibration_passed: bool,
    pub suppression_reasons: Vec<String>,
    pub out_of_domain_warnings: Vec<String>,
    pub evidence_coverage: Option<EvidenceCoverage>,
    pub critical_claims: u64,
    pub bound_critical_claims: u64,
    pub first_pass_success_rate: Option<f64>,
    pub mean_attempts_to_green: Option<f64>,
    pub flake_rate: Option<f64>,
    pub human_escalation_rate: Option<f64>,
    pub cost_usd: Option<f64>,
    pub elapsed_ms: Option<u64>,
    pub raw_sample_size: u64,
    pub effective_sample_size: f64,
    pub versions: ScoreVersions,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn task_family(packet: &NextPassPacket) -> String {
    let families: BTreeSet<&str> = packet
        .evidence
        .iter()
        .filter(|record| {
            !matches!(
                record.kind.as_str(),
                "objective" | "subagent-session" | "codex-synthesis" | "unstructured"
            )
        })
        .map(|record| record.kind.as_str())
        .collect();
    if families.is_empty() {
        "unclassified".to_string()
    } else {
        families.into_iter().collect::<Vec<_>>().join("+")
    }
}

fn exact_agent_variant(provider: &str, model: &str, agent: &str, version: &str) -> String {
    format!("{provider}:{model}:{agent}:{version}")
}

fn categorical_reasons(packet: &NextPassPacket) -> Vec<String> {
    let mut reasons = Vec::new();
    if packet.trust_assessments.is_empty() {
        reasons.push("no critical claims have typed trust assessments".to_string());
    }
    for assessment in &packet.trust_assessments {
        if matches!(
            assessment.applicability.as_str(),
            "stale" | "environment_mismatch" | "unbound" | "unknown"
        ) {
            reasons.push(format!(
                "critical claim {} is {}",
                assessment.subject_id, assessment.applicability
            ));
        }
        if assessment.integrity == "invalid"
            || assessment.outcome == "failed"
            || matches!(assessment.claim_status.as_str(), "refuted" | "unknown")
        {
            reasons.push(format!(
                "critical claim {} is categorically {} / {} / {}",
                assessment.subject_id,
                assessment.integrity,
                assessment.outcome,
                assessment.claim_status
            ));
        }
    }
    reasons.sort();
    reasons.dedup();
    reasons
}

fn completion_from_beta(alpha: f64, beta: f64) -> CompletionInterval {
    CompletionInterval {
        rate: beta / (alpha + beta),
        lower_95: 1.0 - beta_quantile(0.975, alpha, beta),
        upper_95: 1.0 - beta_quantile(0.025, alpha, beta),
    }
}

fn completion_from_draws(risk_draws: &[f64]) -> CompletionInterval {
    let mut completion: Vec<f64> = risk_draws.iter().map(|value| 1.0 - value).collect();
    completion.sort_by(f64::total_cmp);
    CompletionInterval {
        rate: completion.iter().sum::<f64>() / completion.len() as f64,
        lower_95: percentile(&completion, 0.025),
        upper_95: percentile(&completion, 0.975),
    }
}

fn ood_warnings(bundle: &CalibrationBundle, features: &RuntimeFeatures) -> Vec<String> {
    let values = [
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
    let mut warnings = Vec::new();
    for (feature, value) in values {
        if bundle
            .valid_domains
            .get(feature)
            .is_none_or(|domain| !domain.contains(&value))
        {
            warnings.push(format!(
                "{feature} `{value}` is outside the calibration domain"
            ));
        }
    }
    warnings
}

pub fn score_run(
    run_dir: &Path,
    bundle_path: &Path,
) -> Result<ReliabilityScore, Box<dyn std::error::Error>> {
    compile_run_dir(run_dir)?;
    let manifest: RunManifest = read_json(&run_dir.join("manifest.json"))?;
    let packet: NextPassPacket = read_json(&run_dir.join("state/next_pass_packet.json"))?;
    let session = latest_session(run_dir)?;
    let bundle = load_verified_bundle(bundle_path)?;
    let family = task_family(&packet);
    let suppression_reasons = categorical_reasons(&packet);
    let repo_root = PathBuf::from(
        manifest
            .repo_root
            .as_deref()
            .ok_or("score requires repo_root in manifest.json")?,
    );
    let attempts = load_verified_attempts(run_dir)?;
    let check_versions: BTreeSet<String> = attempts
        .iter()
        .map(|attempt| format!("{}@{}", attempt.check_id, attempt.check_version))
        .collect();
    let total_attempts: u32 = packet
        .check_histories
        .iter()
        .map(|history| history.attempts)
        .sum();
    let first_passes = packet
        .check_histories
        .iter()
        .filter(|history| matches!(history.first_result.as_str(), "passed" | "expected_failure"))
        .count();
    let attempts_to_green: Vec<u32> = packet
        .check_histories
        .iter()
        .filter_map(|history| history.attempts_to_green)
        .collect();
    let flake_rate = if packet.check_histories.is_empty() {
        None
    } else {
        Some(
            packet
                .check_histories
                .iter()
                .map(|history| history.flake_rate)
                .sum::<f64>()
                / packet.check_histories.len() as f64,
        )
    };
    let negative_control_status = if attempts
        .iter()
        .any(|attempt| attempt.negative_control_outcome.as_deref() == Some("expected_failure"))
    {
        "passed"
    } else if attempts
        .iter()
        .any(|attempt| attempt.negative_control_outcome.is_some())
    {
        "failed"
    } else {
        "not_run"
    };
    let independent_outcomes = if run_dir.join("outcomes/outcomes.jsonl").exists() {
        load_outcome_records(run_dir)?
            .into_iter()
            .map(|(outcome, _)| outcome)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let human_escalation_rate = if independent_outcomes.is_empty() {
        None
    } else {
        Some(
            independent_outcomes
                .iter()
                .filter(|outcome| outcome.adjudicator.role == "human-review")
                .count() as f64
                / independent_outcomes.len() as f64,
        )
    };
    let judge_identities: BTreeSet<String> = independent_outcomes
        .iter()
        .map(|outcome| outcome.adjudicator.principal_id.clone())
        .collect();
    let verifier_independent = independent_outcomes
        .last()
        .is_some_and(|outcome| outcome.adjudicator.independent_from_worker);
    let change = crate::compiler::outcomes::change_size(&repo_root);
    let agent_variant = match (
        session.provider.as_deref(),
        session.resolved_model_snapshot.as_deref(),
        session.agent_name.as_deref(),
        session.agent_version.as_deref(),
    ) {
        (Some(provider), Some(model), Some(agent), Some(version)) => {
            Some(exact_agent_variant(provider, model, agent, version))
        }
        _ => None,
    };
    let features = RuntimeFeatures {
        agent_variant: agent_variant
            .clone()
            .unwrap_or_else(|| "unresolved".to_string()),
        task_family: family.clone(),
        repository_id: repository_id(&repo_root),
        language: language(&repo_root),
        freshness: freshness(&packet),
        environment_match: if packet
            .trust_assessments
            .iter()
            .any(|assessment| assessment.applicability == "environment_mismatch")
        {
            "mismatch".to_string()
        } else if packet.trust_assessments.is_empty() {
            "unknown".to_string()
        } else {
            "matched".to_string()
        },
        negative_control_status: negative_control_status.to_string(),
        verifier_independent,
        verification_strength: attempts.len() as f64
            + if negative_control_status == "passed" {
                1.0
            } else {
                0.0
            },
        attempts: f64::from(total_attempts),
        flakiness: flake_rate.unwrap_or(0.0),
        change_size: f64::from(change.files + change.additions + change.deletions),
    };
    // Baseline eligibility is an exact cohort lookup. Its domain dictionary is
    // intentionally smaller than the hierarchical feature dictionary, so
    // applying hierarchical OOD checks here would manufacture false warnings.
    let mut warnings = if bundle.model_kind == "hierarchical" {
        ood_warnings(&bundle, &features)
    } else {
        Vec::new()
    };
    let mut risk = None;
    let mut upper = None;
    let mut interval_width = None;
    let mut completion = None;
    let mut raw_sample_size = 0;
    let mut effective_sample_size = 0.0;
    let mut calibration_status = bundle.publication_state.clone();
    if bundle.model_kind == "baseline" {
        let cohort = bundle.cohorts.iter().find(|cohort| {
            agent_variant.as_ref().is_some_and(|variant| {
                *variant
                    == exact_agent_variant(
                        &cohort.cohort.provider,
                        &cohort.cohort.model_snapshot,
                        &cohort.cohort.agent_name,
                        &cohort.cohort.agent_version,
                    )
            }) && cohort.cohort.task_family == family
        });
        if let Some(cohort) = cohort {
            raw_sample_size = cohort.raw_outcomes;
            effective_sample_size = cohort.effective_outcomes;
            calibration_status = cohort.publication_state.clone();
            if suppression_reasons.is_empty() {
                risk = cohort.posterior.false_green_probability;
                upper = cohort.posterior.upper_95_false_green_risk;
                interval_width = cohort.posterior.interval_95_width;
                if let (Some(alpha), Some(beta)) = (
                    cohort.posterior.posterior_alpha,
                    cohort.posterior.posterior_beta,
                ) {
                    completion = Some(completion_from_beta(alpha, beta));
                }
            }
        } else {
            warnings.push("no exact model-agent-task-family cohort exists in bundle".to_string());
            calibration_status = "insufficient_data".to_string();
        }
    } else if suppression_reasons.is_empty() {
        let draws = runtime_hierarchical_draws(&bundle, &features)?;
        let mut sorted = draws.clone();
        sorted.sort_by(f64::total_cmp);
        risk = Some(draws.iter().sum::<f64>() / draws.len() as f64);
        upper = Some(percentile(&sorted, 0.95));
        interval_width = Some(percentile(&sorted, 0.975) - percentile(&sorted, 0.025));
        completion = Some(completion_from_draws(&draws));
        raw_sample_size = bundle.sample_counts.raw_outcomes;
        effective_sample_size = bundle.sample_counts.effective_outcomes;
    }
    let score_status = if !suppression_reasons.is_empty() {
        "suppressed"
    } else if risk.is_none() {
        "insufficient_data"
    } else if !warnings.is_empty() {
        "out_of_domain"
    } else {
        calibration_status.as_str()
    };
    let elapsed_ms = Some(
        load_verified_receipts(run_dir)?
            .iter()
            .map(|receipt| receipt.duration_ms)
            .sum(),
    );
    let critical_claims = packet.trust_assessments.len() as u64;
    let bound_critical_claims = packet
        .trust_assessments
        .iter()
        .filter(|assessment| {
            assessment.applicability == "current"
                && matches!(
                    assessment.claim_status.as_str(),
                    "verified" | "verifier_backed"
                )
        })
        .count() as u64;
    Ok(ReliabilityScore {
        format_version: "1".to_string(),
        run_id: manifest.run_id,
        objective_id: manifest.objective_id,
        score_status: score_status.to_string(),
        calibration_status,
        task_family: family,
        false_green_probability: risk,
        upper_95_false_green_risk: upper,
        false_green_interval_95_width: interval_width,
        verified_completion: completion,
        held_out_calibration_passed: bundle.publication_state == "calibrated",
        suppression_reasons,
        out_of_domain_warnings: warnings,
        evidence_coverage: packet.evidence_coverage,
        critical_claims,
        bound_critical_claims,
        first_pass_success_rate: if packet.check_histories.is_empty() {
            None
        } else {
            Some(first_passes as f64 / packet.check_histories.len() as f64)
        },
        mean_attempts_to_green: if attempts_to_green.is_empty() {
            None
        } else {
            Some(
                attempts_to_green
                    .iter()
                    .map(|value| f64::from(*value))
                    .sum::<f64>()
                    / attempts_to_green.len() as f64,
            )
        },
        flake_rate,
        human_escalation_rate,
        cost_usd: None,
        elapsed_ms,
        raw_sample_size,
        effective_sample_size,
        versions: ScoreVersions {
            provider: session.provider,
            requested_model: session.requested_model,
            resolved_model_snapshot: session.resolved_model_snapshot,
            agent_name: session.agent_name,
            agent_version: session.agent_version,
            scaffold_name: session.scaffold_name,
            scaffold_version: session.scaffold_version,
            engine_version: session.engine.engine_version,
            engine_build_commit: session.engine.build_commit,
            engine_binary_digest: session.engine.binary_digest.digest,
            check_versions: check_versions.into_iter().collect(),
            harness_versions: attempts
                .iter()
                .map(|attempt| format!("{}@{}", attempt.check_id, attempt.check_version))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            judge_identities: judge_identities.into_iter().collect(),
            dataset_hash: bundle.dataset_hash,
            calibration_bundle_hash: bundle.bundle_hash,
            methodology_version: bundle.methodology_version,
        },
    })
}
