//! Engine-owned check definitions, execution, and freshness bindings.
//!
//! Check commands come from `<repo>/.receipts/checks.toml`, never agent prose.
//! Every attempt records the exact covered file bytes, dependency locks,
//! environment, command/version, target claims, and receipt IDs in a chained
//! sidecar. Compile recomputes those bindings, so an old green cannot survive
//! a relevant subject, lock, environment, or check-definition change.

use crate::compiler::receipts::{append_receipt, store_artifact};
use crate::schema::{CheckHistory, ReceiptRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const ATTEMPT_FORMAT_VERSION: &str = "1";
const GENESIS: &str = "GENESIS";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckManifest {
    pub manifest_version: u32,
    pub checks: Vec<CheckDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDefinition {
    pub id: String,
    pub version: String,
    pub command: Vec<String>,
    pub covered_paths: Vec<String>,
    pub eligible_claim_kinds: Vec<String>,
    pub environment_class: String,
    pub target_claims: Vec<String>,
    #[serde(default)]
    pub negative_control_command: Option<Vec<String>>,
    #[serde(default)]
    pub negative_control_expected_signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckAttemptRecord {
    pub format_version: String,
    pub id: String,
    pub check_id: String,
    pub check_version: String,
    pub target_claims: Vec<String>,
    pub covered_paths: Vec<String>,
    pub subject_digest: String,
    pub dependency_lock_digest: String,
    pub command_digest: String,
    pub definition_digest: String,
    pub environment_class: String,
    pub environment_digest: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_signature: Option<String>,
    pub primary_receipt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_control_receipt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_control_outcome: Option<String>,
    pub attempted_at: String,
    pub prev_record_hash: String,
    pub record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimBinding {
    pub applicability: String,
    pub outcome: String,
    pub check_id: Option<String>,
}

pub fn load_manifest(
    repo_root: &Path,
) -> Result<Option<CheckManifest>, Box<dyn std::error::Error>> {
    let path = repo_root.join(".receipts").join("checks.toml");
    if !path.exists() {
        return Ok(None);
    }
    let manifest: CheckManifest = toml::from_str(&fs::read_to_string(&path)?)?;
    validate_manifest(&manifest)?;
    Ok(Some(manifest))
}

fn validate_manifest(manifest: &CheckManifest) -> Result<(), Box<dyn std::error::Error>> {
    if manifest.manifest_version != 1 {
        return Err(format!(
            "unsupported checks.toml manifest_version {} (expected 1)",
            manifest.manifest_version
        )
        .into());
    }
    let mut ids = BTreeSet::new();
    for check in &manifest.checks {
        if check.id.is_empty()
            || !check
                .id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '.' | '_' | '/' | '-'))
        {
            return Err(format!("invalid check id `{}`", check.id).into());
        }
        if !ids.insert(&check.id) {
            return Err(format!("duplicate check id `{}`", check.id).into());
        }
        if check.version.is_empty()
            || check.command.is_empty()
            || check.covered_paths.is_empty()
            || check.eligible_claim_kinds.is_empty()
            || check.environment_class.is_empty()
            || check.target_claims.is_empty()
        {
            return Err(format!("check `{}` has an empty required field", check.id).into());
        }
        for pattern in &check.covered_paths {
            let path = Path::new(pattern);
            if path.is_absolute()
                || path
                    .components()
                    .any(|part| matches!(part, std::path::Component::ParentDir))
            {
                return Err(format!(
                    "check `{}` covered path `{pattern}` escapes repo_root",
                    check.id
                )
                .into());
            }
        }
        match (
            &check.negative_control_command,
            &check.negative_control_expected_signature,
        ) {
            (Some(command), Some(signature)) if !command.is_empty() && !signature.is_empty() => {}
            (None, None) => {}
            _ => {
                return Err(format!(
                    "check `{}` must declare both a tokenized negative_control_command and negative_control_expected_signature",
                    check.id
                )
                .into());
            }
        }
    }
    Ok(())
}

pub fn find_check<'a>(
    manifest: &'a CheckManifest,
    id: &str,
) -> Result<&'a CheckDefinition, Box<dyn std::error::Error>> {
    manifest
        .checks
        .iter()
        .find(|check| check.id == id)
        .ok_or_else(|| format!("check `{id}` is not declared in .receipts/checks.toml").into())
}

pub fn run_check(
    run_dir: &Path,
    repo_root: &Path,
    check: &CheckDefinition,
    writer: &str,
) -> Result<CheckAttemptRecord, Box<dyn std::error::Error>> {
    let binding = compute_binding(repo_root, check)?;
    let (primary, primary_output) = execute_receipted(
        run_dir,
        repo_root,
        Some(format!("check:{}", check.id)),
        &check.command,
        writer,
    )?;
    let mut outcome = if primary.exit_code == 0 {
        "passed"
    } else {
        "failed"
    }
    .to_string();
    let mut failure_signature = if primary.exit_code == 0 {
        None
    } else {
        Some(output_signature(&primary_output))
    };
    let mut negative_control_receipt_id = None;
    let mut negative_control_outcome = None;

    if primary.exit_code == 0 {
        if let (Some(command), Some(expected)) = (
            &check.negative_control_command,
            &check.negative_control_expected_signature,
        ) {
            let (control, output) = execute_receipted(
                run_dir,
                repo_root,
                Some(format!("check:{}:negative-control", check.id)),
                command,
                writer,
            )?;
            negative_control_receipt_id = Some(control.id.clone());
            let combined = String::from_utf8_lossy(&output).to_string();
            if control.exit_code != 0 && combined.contains(expected) {
                negative_control_outcome = Some("expected_failure".to_string());
            } else {
                negative_control_outcome = Some("failed".to_string());
                outcome = "failed".to_string();
                failure_signature = Some(output_signature(&output));
            }
        }
    }

    append_attempt(
        run_dir,
        CheckAttemptRecord {
            format_version: ATTEMPT_FORMAT_VERSION.to_string(),
            id: String::new(),
            check_id: check.id.clone(),
            check_version: check.version.clone(),
            target_claims: check.target_claims.clone(),
            covered_paths: binding.covered_paths,
            subject_digest: binding.subject_digest,
            dependency_lock_digest: binding.dependency_lock_digest,
            command_digest: binding.command_digest,
            definition_digest: binding.definition_digest,
            environment_class: check.environment_class.clone(),
            environment_digest: binding.environment_digest,
            outcome,
            failure_signature,
            primary_receipt_id: primary.id,
            negative_control_receipt_id,
            negative_control_outcome,
            attempted_at: iso_now(),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        },
    )
}

struct BindingSnapshot {
    covered_paths: Vec<String>,
    subject_digest: String,
    dependency_lock_digest: String,
    command_digest: String,
    definition_digest: String,
    environment_digest: String,
}

fn compute_binding(
    repo_root: &Path,
    check: &CheckDefinition,
) -> Result<BindingSnapshot, Box<dyn std::error::Error>> {
    let mut matches = BTreeSet::new();
    let root = repo_root.canonicalize()?;
    let root_pattern = root.to_string_lossy().replace('\\', "/");
    for covered in &check.covered_paths {
        let pattern = format!("{root_pattern}/{covered}");
        for entry in glob::glob(&pattern)
            .map_err(|err| format!("invalid covered path glob `{covered}`: {err}"))?
        {
            let path = entry?;
            if path.is_file() {
                let canonical = path.canonicalize()?;
                if !canonical.starts_with(&root) {
                    return Err(
                        format!("covered path `{}` escapes repo_root", path.display()).into(),
                    );
                }
                matches.insert(canonical);
            }
        }
    }
    if matches.is_empty() {
        return Err(format!("check `{}` covered_paths matched zero files", check.id).into());
    }
    let mut subject = blake3::Hasher::new();
    let mut covered_paths = Vec::new();
    for path in matches {
        let rel = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        covered_paths.push(rel.clone());
        subject.update(rel.as_bytes());
        subject.update(&[0]);
        subject.update(&fs::read(&path)?);
        subject.update(&[0]);
    }

    let lock_names = [
        "Cargo.lock",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "uv.lock",
    ];
    let mut locks = blake3::Hasher::new();
    let mut found_lock = false;
    for name in lock_names {
        let path = root.join(name);
        if path.is_file() {
            found_lock = true;
            locks.update(name.as_bytes());
            locks.update(&[0]);
            locks.update(&fs::read(path)?);
            locks.update(&[0]);
        }
    }
    if !found_lock {
        locks.update(b"no-root-dependency-lock");
    }

    let command = serde_json::to_vec(&check.command)?;
    let definition = serde_json::to_vec(check)?;
    let environment = format!(
        "{}\0{}\0{}",
        check.environment_class,
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    Ok(BindingSnapshot {
        covered_paths,
        subject_digest: subject.finalize().to_hex().to_string(),
        dependency_lock_digest: locks.finalize().to_hex().to_string(),
        command_digest: blake3::hash(&command).to_hex().to_string(),
        definition_digest: blake3::hash(&definition).to_hex().to_string(),
        environment_digest: blake3::hash(environment.as_bytes()).to_hex().to_string(),
    })
}

fn execute_receipted(
    run_dir: &Path,
    repo_root: &Path,
    label: Option<String>,
    command: &[String],
    writer: &str,
) -> Result<(ReceiptRecord, Vec<u8>), Box<dyn std::error::Error>> {
    let started_at = iso_now();
    let start = std::time::Instant::now();
    let output = std::process::Command::new(&command[0])
        .args(&command[1..])
        .current_dir(repo_root)
        .output()
        .map_err(|err| format!("failed to launch check command `{}`: {err}", command[0]))?;
    let duration_ms = start.elapsed().as_millis() as u64;
    let ended_at = iso_now();
    let exit_code = i64::from(output.status.code().unwrap_or(-1));
    let (stdout_hash, _) = store_artifact(run_dir, &output.stdout)?;
    let (stderr_hash, _) = store_artifact(run_dir, &output.stderr)?;
    let bounded_tail = |bytes: &[u8]| -> String {
        String::from_utf8_lossy(bytes)
            .chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    };
    let mut combined = output.stdout.clone();
    combined.extend_from_slice(&output.stderr);
    let receipt = append_receipt(
        run_dir,
        ReceiptRecord {
            id: String::new(),
            label,
            cmd: command.to_vec(),
            cwd: repo_root.to_string_lossy().to_string(),
            exit_code,
            duration_ms,
            started_at,
            ended_at,
            stdout_hash,
            stderr_hash,
            stdout_tail: bounded_tail(&output.stdout),
            stderr_tail: bounded_tail(&output.stderr),
            tree_before: None,
            tree_after: None,
            lane: Some("check-engine".to_string()),
            agent_id: Some("receipts-engine".to_string()),
            writer: writer.to_string(),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        },
    )?;
    Ok((receipt, combined))
}

fn output_signature(output: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(output).to_hex())
}

fn attempt_path(run_dir: &Path) -> PathBuf {
    run_dir.join("checks").join("attempts.jsonl")
}

fn attempt_hash(attempt: &CheckAttemptRecord) -> Result<String, Box<dyn std::error::Error>> {
    let mut canonical = attempt.clone();
    canonical.record_hash.clear();
    Ok(blake3::hash(&serde_json::to_vec(&canonical)?)
        .to_hex()
        .to_string())
}

fn append_attempt(
    run_dir: &Path,
    mut attempt: CheckAttemptRecord,
) -> Result<CheckAttemptRecord, Box<dyn std::error::Error>> {
    let path = attempt_path(run_dir);
    fs::create_dir_all(path.parent().expect("attempt parent"))?;
    let existing = load_verified_attempts(run_dir)?;
    attempt.id = format!("check-attempt-{:04}", existing.len() + 1);
    attempt.prev_record_hash = existing
        .last()
        .map(|record| record.record_hash.clone())
        .unwrap_or_else(|| GENESIS.to_string());
    attempt.record_hash = attempt_hash(&attempt)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, &attempt)?;
    file.write_all(b"\n")?;
    Ok(attempt)
}

pub fn load_verified_attempts(
    run_dir: &Path,
) -> Result<Vec<CheckAttemptRecord>, Box<dyn std::error::Error>> {
    let path = attempt_path(run_dir);
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut attempts = Vec::new();
    let mut previous = GENESIS.to_string();
    for (index, line) in fs::read_to_string(path)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let attempt: CheckAttemptRecord = serde_json::from_str(line)
            .map_err(|err| format!("checks/attempts.jsonl line {} is invalid: {err}", index + 1))?;
        let actual = attempt_hash(&attempt)?;
        if attempt.format_version != ATTEMPT_FORMAT_VERSION
            || attempt.record_hash != actual
            || attempt.prev_record_hash != previous
        {
            return Err(format!(
                "check attempt chain broken at entry {} ({})",
                index + 1,
                attempt.id
            )
            .into());
        }
        previous = attempt.record_hash.clone();
        attempts.push(attempt);
    }
    Ok(attempts)
}

pub fn build_check_histories(attempts: &[CheckAttemptRecord]) -> Vec<CheckHistory> {
    let mut by_check: BTreeMap<&str, Vec<&CheckAttemptRecord>> = BTreeMap::new();
    for attempt in attempts {
        by_check.entry(&attempt.check_id).or_default().push(attempt);
    }
    by_check
        .into_iter()
        .filter_map(|(check_id, records)| {
            let first = records.first()?;
            let latest = records.last()?;
            let attempts_to_green = records
                .iter()
                .position(|attempt| attempt.outcome == "passed")
                .map(|index| index as u32 + 1);
            let failure_signatures = records
                .iter()
                .filter_map(|attempt| attempt.failure_signature.clone())
                .collect();
            let transitions = records
                .windows(2)
                .filter_map(|pair| {
                    (pair[0].outcome != pair[1].outcome)
                        .then(|| format!("{}->{}", pair[0].outcome, pair[1].outcome))
                })
                .collect();
            let failed = records
                .iter()
                .filter(|attempt| attempt.outcome != "passed")
                .count();
            Some(CheckHistory {
                check_id: check_id.to_string(),
                target_claims: latest.target_claims.clone(),
                first_result: first.outcome.clone(),
                latest_result: latest.outcome.clone(),
                attempts: records.len() as u32,
                attempts_to_green,
                failure_signatures,
                transitions,
                flake_rate: failed as f64 / records.len() as f64,
            })
        })
        .collect()
}

pub fn verify_attempt_receipts(
    attempts: &[CheckAttemptRecord],
    receipts: &[ReceiptRecord],
) -> Result<(), Box<dyn std::error::Error>> {
    for attempt in attempts {
        let primary = receipts
            .iter()
            .find(|receipt| receipt.id == attempt.primary_receipt_id)
            .ok_or_else(|| {
                format!(
                    "check attempt {} does not reference a verified primary receipt {} in this run",
                    attempt.id, attempt.primary_receipt_id
                )
            })?;
        let expected_label = format!("check:{}", attempt.check_id);
        let command_digest = blake3::hash(&serde_json::to_vec(&primary.cmd)?)
            .to_hex()
            .to_string();
        if primary.label.as_deref() != Some(expected_label.as_str())
            || command_digest != attempt.command_digest
        {
            return Err(format!(
                "check attempt {} primary receipt {} does not match its check label and command",
                attempt.id, primary.id
            )
            .into());
        }
        if attempt.outcome == "passed" && primary.exit_code != 0 {
            return Err(format!(
                "check attempt {} claims passed but primary receipt {} exited {}",
                attempt.id, primary.id, primary.exit_code
            )
            .into());
        }

        match (
            attempt.negative_control_receipt_id.as_deref(),
            attempt.negative_control_outcome.as_deref(),
        ) {
            (None, None) => {}
            (Some(receipt_id), Some(control_outcome)) => {
                let control = receipts
                    .iter()
                    .find(|receipt| receipt.id == receipt_id)
                    .ok_or_else(|| {
                        format!(
                            "check attempt {} does not reference verified negative-control receipt {} in this run",
                            attempt.id, receipt_id
                        )
                    })?;
                let expected_control_label = format!("check:{}:negative-control", attempt.check_id);
                if control.label.as_deref() != Some(expected_control_label.as_str()) {
                    return Err(format!(
                        "check attempt {} negative-control receipt {} has the wrong label",
                        attempt.id, control.id
                    )
                    .into());
                }
                if control_outcome == "expected_failure" && control.exit_code == 0 {
                    return Err(format!(
                        "check attempt {} claims expected_failure but negative control exited 0",
                        attempt.id
                    )
                    .into());
                }
            }
            _ => {
                return Err(format!(
                    "check attempt {} has an incomplete negative-control binding",
                    attempt.id
                )
                .into());
            }
        }
    }
    Ok(())
}

pub fn claim_binding(
    repo_root: Option<&Path>,
    claim_id: &str,
    claim_kind: &str,
    attempts: &[CheckAttemptRecord],
) -> Result<ClaimBinding, Box<dyn std::error::Error>> {
    let Some(repo_root) = repo_root else {
        return Ok(unbound());
    };
    let Some(manifest) = load_manifest(repo_root)? else {
        return Ok(unbound());
    };
    let eligible: Vec<&CheckDefinition> = manifest
        .checks
        .iter()
        .filter(|check| {
            check.target_claims.iter().any(|id| id == claim_id)
                && check
                    .eligible_claim_kinds
                    .iter()
                    .any(|kind| kind == claim_kind)
        })
        .collect();
    if eligible.is_empty() {
        return Ok(unbound());
    }
    let mut saw_stale = false;
    let mut saw_environment_mismatch = false;
    for check in eligible.into_iter().rev() {
        let Some(attempt) = attempts
            .iter()
            .rev()
            .find(|attempt| attempt.check_id == check.id)
        else {
            continue;
        };
        let current = match compute_binding(repo_root, check) {
            Ok(current) => current,
            Err(_) => {
                saw_stale = true;
                continue;
            }
        };
        if attempt.environment_class != check.environment_class
            || attempt.environment_digest != current.environment_digest
        {
            saw_environment_mismatch = true;
            continue;
        }
        if attempt.check_version != check.version
            || attempt.target_claims != check.target_claims
            || attempt.covered_paths != current.covered_paths
            || attempt.subject_digest != current.subject_digest
            || attempt.dependency_lock_digest != current.dependency_lock_digest
            || attempt.command_digest != current.command_digest
            || attempt.definition_digest != current.definition_digest
        {
            saw_stale = true;
            continue;
        }
        return Ok(ClaimBinding {
            applicability: "current".to_string(),
            outcome: attempt.outcome.clone(),
            check_id: Some(check.id.clone()),
        });
    }
    Ok(ClaimBinding {
        applicability: if saw_environment_mismatch {
            "environment_mismatch"
        } else if saw_stale {
            "stale"
        } else {
            "unbound"
        }
        .to_string(),
        outcome: "unknown".to_string(),
        check_id: None,
    })
}

fn unbound() -> ClaimBinding {
    ClaimBinding {
        applicability: "unbound".to_string(),
        outcome: "unknown".to_string(),
        check_id: None,
    }
}

fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix:{seconds}")
}
