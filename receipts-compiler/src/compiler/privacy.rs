//! Deterministic, allowlist-only public projection.
//!
//! This module never redacts a copy of the private packet. It constructs a
//! new aggregate document from fixed fields, so newly added private fields
//! cannot become public by default.

use crate::compiler::receipts::load_verified_receipt_journal;
use crate::compiler::run_dir::compile_run_dir;
use crate::schema::{EvidenceCoverage, NextPassPacket};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct PublicEngineIdentity {
    pub protocol_version: String,
    pub engine_version: String,
    pub build_commit: String,
    pub binary_digest: String,
    pub dependency_lock_digest: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicClaimSummary {
    pub total: u32,
    pub by_status: BTreeMap<String, u32>,
    pub by_applicability: BTreeMap<String, u32>,
    pub independently_verified: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicReceiptSummary {
    pub total: u32,
    pub by_integrity: BTreeMap<String, u32>,
    pub by_outcome: BTreeMap<String, u32>,
    pub attempts_total: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicCheckSummary {
    pub checks: u32,
    pub attempts: u32,
    pub first_pass_successes: u32,
    pub eventually_green: u32,
    pub flaky_checks: u32,
    pub transitions: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PublicProjection {
    pub projection_version: String,
    pub schema_version: String,
    pub engines: Vec<PublicEngineIdentity>,
    pub executor_principals: Vec<String>,
    pub evidence_coverage: EvidenceCoverage,
    pub claims: PublicClaimSummary,
    pub receipts: PublicReceiptSummary,
    pub checks: PublicCheckSummary,
}

fn initialized_counts(keys: &[&str]) -> BTreeMap<String, u32> {
    keys.iter().map(|key| ((*key).to_string(), 0)).collect()
}

fn increment_known(counts: &mut BTreeMap<String, u32>, value: &str) {
    let key = if counts.contains_key(value) {
        value
    } else {
        "unknown"
    };
    *counts.entry(key.to_string()).or_default() += 1;
}

pub fn build_public_projection(
    run_dir: &Path,
) -> Result<PublicProjection, Box<dyn std::error::Error>> {
    // Recompile from the on-disk private inputs first. A public projection is
    // never allowed to bless a stale packet left by an earlier run.
    compile_run_dir(run_dir)?;
    let packet: NextPassPacket = serde_json::from_slice(&fs::read(
        run_dir.join("state").join("next_pass_packet.json"),
    )?)?;
    let journal = load_verified_receipt_journal(run_dir)?;

    let mut engines = BTreeSet::new();
    let mut principals = BTreeSet::new();
    for verification in journal.verification.values() {
        if let Some(engine) = &verification.engine {
            engines.insert(PublicEngineIdentity {
                protocol_version: engine.protocol_version.clone(),
                engine_version: engine.engine_version.clone(),
                build_commit: engine.build_commit.clone(),
                binary_digest: engine.binary_digest.digest.clone(),
                dependency_lock_digest: engine.dependency_lock_digest.digest.clone(),
                os: engine.os.clone(),
                arch: engine.arch.clone(),
            });
        }
        if let Some(principal) = &verification.principal_id {
            principals.insert(principal.clone());
        }
    }

    let mut claim_status = initialized_counts(&[
        "verified",
        "verifier_backed",
        "asserted",
        "refuted",
        "unknown",
    ]);
    let mut applicability = initialized_counts(&[
        "current",
        "stale",
        "environment_mismatch",
        "unbound",
        "unknown",
    ]);
    let mut independently_verified = 0;
    for assessment in &packet.trust_assessments {
        increment_known(&mut claim_status, &assessment.claim_status);
        increment_known(&mut applicability, &assessment.applicability);
        if assessment.verifier_independent && assessment.claim_status == "verified" {
            independently_verified += 1;
        }
    }

    let mut receipt_integrity = initialized_counts(&[
        "signed",
        "hash_verified",
        "legacy_weak",
        "invalid",
        "unknown",
    ]);
    let mut receipt_outcome =
        initialized_counts(&["passed", "failed", "expected_failure", "unknown"]);
    let mut receipt_attempts = 0;
    for event in &packet.receipt_events {
        increment_known(&mut receipt_integrity, &event.integrity);
        increment_known(&mut receipt_outcome, &event.outcome);
        receipt_attempts += event.attempts_for_label;
    }

    let check_summary = PublicCheckSummary {
        checks: packet.check_histories.len() as u32,
        attempts: packet
            .check_histories
            .iter()
            .map(|history| history.attempts)
            .sum(),
        first_pass_successes: packet
            .check_histories
            .iter()
            .filter(|history| history.first_result == "passed")
            .count() as u32,
        eventually_green: packet
            .check_histories
            .iter()
            .filter(|history| history.latest_result == "passed")
            .count() as u32,
        flaky_checks: packet
            .check_histories
            .iter()
            .filter(|history| history.flake_rate > 0.0)
            .count() as u32,
        transitions: packet
            .check_histories
            .iter()
            .map(|history| history.transitions.len() as u32)
            .sum(),
    };

    Ok(PublicProjection {
        projection_version: "1.0.0".to_string(),
        schema_version: packet.schema_version,
        engines: engines.into_iter().collect(),
        executor_principals: principals.into_iter().collect(),
        evidence_coverage: packet.evidence_coverage.unwrap_or_default(),
        claims: PublicClaimSummary {
            total: packet.trust_assessments.len() as u32,
            by_status: claim_status,
            by_applicability: applicability,
            independently_verified,
        },
        receipts: PublicReceiptSummary {
            total: packet.receipt_events.len() as u32,
            by_integrity: receipt_integrity,
            by_outcome: receipt_outcome,
            attempts_total: receipt_attempts,
        },
        checks: check_summary,
    })
}

pub fn write_public_projection(
    run_dir: &Path,
    out: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let projection = build_public_projection(run_dir)?;
    let mut bytes = serde_json::to_vec_pretty(&projection)?;
    bytes.push(b'\n');
    let text = String::from_utf8(bytes.clone())?;
    for forbidden in [
        "\\\\",
        "file:",
        "http://",
        "https://",
        "repo_root",
        "objective",
        "stdout",
        "stderr",
        "prompt",
        "source_text",
        "source_ids",
        "\"cmd\"",
    ] {
        if text.to_ascii_lowercase().contains(forbidden) {
            return Err(format!(
                "public projection safety scanner rejected forbidden material `{forbidden}`"
            )
            .into());
        }
    }
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, bytes)?;
    Ok(())
}
