//! Signed, independently cited task outcomes.

use crate::compiler::checks::load_verified_attempts;
use crate::compiler::crypto::current_executor_identity;
use crate::compiler::run_dir::{RunManifest, compile_run_dir};
use crate::compiler::session::{SessionCapture, latest_session};
use crate::compiler::signed_journal::{
    SignedJournalRecord, append_signed_record, load_verified_signed_records,
};
use crate::schema::{CheckHistory, NextPassPacket};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const JOURNAL: &str = "outcomes/outcomes.jsonl";
const KIND: &str = "independent_outcome";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutcomeCitation {
    pub cite: String,
    pub kind: String,
    pub hash_alg: String,
    pub digest: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutcomeModelIdentity {
    pub session_capture_id: String,
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model_snapshot: Option<String>,
    pub resolution_status: String,
    pub model_specific_eligible: bool,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub scaffold_name: Option<String>,
    pub scaffold_version: Option<String>,
    pub tool_configuration_digest: Option<String>,
    pub reasoning_setting_digest: Option<String>,
    pub environment_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdjudicatorIdentity {
    pub principal_id: String,
    pub authenticated_by: String,
    pub role: String,
    pub independent_from_worker: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChangeSize {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IndependentOutcome {
    pub outcome_id: String,
    pub task_id: String,
    pub claim_ids: Vec<String>,
    pub result: String,
    pub source_grade: String,
    pub training_eligibility: String,
    pub eligibility_reason: String,
    pub observation_window_start: String,
    pub observation_window_end: String,
    pub adjudicator: AdjudicatorIdentity,
    pub citations: Vec<OutcomeCitation>,
    pub model: OutcomeModelIdentity,
    pub task_family: String,
    #[serde(default = "unknown_value")]
    pub repository_id: String,
    pub language: String,
    #[serde(default = "unknown_value")]
    pub freshness: String,
    pub change_size: ChangeSize,
    pub check_strength: String,
    pub environment_match: String,
    pub retry_history: Vec<CheckHistory>,
}

fn unknown_value() -> String {
    "unknown".to_string()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn session_model(session: SessionCapture) -> OutcomeModelIdentity {
    OutcomeModelIdentity {
        session_capture_id: session.session_capture_id,
        provider: session.provider,
        requested_model: session.requested_model,
        resolved_model_snapshot: session.resolved_model_snapshot,
        resolution_status: session.resolution_status,
        model_specific_eligible: session.model_specific_eligible,
        agent_name: session.agent_name,
        agent_version: session.agent_version,
        scaffold_name: session.scaffold_name,
        scaffold_version: session.scaffold_version,
        tool_configuration_digest: session.tool_configuration_digest,
        reasoning_setting_digest: session.reasoning_setting_digest,
        environment_digest: session.environment_digest,
    }
}

fn verify_citation(
    cite: &str,
    repo_root: &Path,
    observed_at: &str,
) -> Result<OutcomeCitation, Box<dyn std::error::Error>> {
    if let Some(declared) = cite.strip_prefix("file:") {
        let candidate = if Path::new(declared).is_absolute() {
            PathBuf::from(declared)
        } else {
            repo_root.join(declared)
        };
        let canonical = fs::canonicalize(candidate)?;
        let root = fs::canonicalize(repo_root)?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err("outcome file citation must resolve to a file inside repo_root".into());
        }
        let relative = canonical
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        return Ok(OutcomeCitation {
            cite: format!("file:{relative}"),
            kind: "file".to_string(),
            hash_alg: "blake3-256".to_string(),
            digest: blake3::hash(&fs::read(canonical)?).to_hex().to_string(),
            observed_at: observed_at.to_string(),
        });
    }
    if let Some(commit) = cite.strip_prefix("commit:") {
        let valid = commit.len() >= 7
            && commit.len() <= 40
            && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
            && std::process::Command::new("git")
                .args([
                    "-C",
                    repo_root.to_string_lossy().as_ref(),
                    "cat-file",
                    "-e",
                    &format!("{commit}^{{commit}}"),
                ])
                .status()?
                .success();
        if !valid {
            return Err(format!("outcome commit citation does not resolve: {cite}").into());
        }
        return Ok(OutcomeCitation {
            cite: cite.to_string(),
            kind: "commit".to_string(),
            hash_alg: "blake3-256".to_string(),
            digest: blake3::hash(cite.as_bytes()).to_hex().to_string(),
            observed_at: observed_at.to_string(),
        });
    }
    Err("outcome citations must be verified file: or commit: anchors".into())
}

pub(crate) fn change_size(repo_root: &Path) -> ChangeSize {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_string_lossy().as_ref(),
            "diff",
            "--numstat",
            "HEAD",
        ])
        .output();
    let Ok(output) = output else {
        return ChangeSize {
            files: 0,
            additions: 0,
            deletions: 0,
            status: "unknown".to_string(),
        };
    };
    if !output.status.success() {
        return ChangeSize {
            files: 0,
            additions: 0,
            deletions: 0,
            status: "unknown".to_string(),
        };
    }
    let mut size = ChangeSize {
        files: 0,
        additions: 0,
        deletions: 0,
        status: "measured".to_string(),
    };
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let columns: Vec<&str> = line.split('\t').collect();
        if columns.len() >= 3 {
            size.files += 1;
            size.additions += columns[0].parse::<u32>().unwrap_or(0);
            size.deletions += columns[1].parse::<u32>().unwrap_or(0);
        }
    }
    size
}

pub(crate) fn repository_id(repo_root: &Path) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_string_lossy().as_ref(),
            "rev-list",
            "--max-parents=0",
            "--reverse",
            "HEAD",
        ])
        .output();
    if let Ok(output) = output {
        if output.status.success() {
            if let Some(root) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .find(|line| line.len() == 40 && line.bytes().all(|byte| byte.is_ascii_hexdigit()))
            {
                return format!("git-root:{}", root.to_ascii_lowercase());
            }
        }
    }
    format!(
        "local:{}",
        blake3::hash(repo_root.to_string_lossy().as_bytes()).to_hex()
    )
}

pub(crate) fn language(repo_root: &Path) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            repo_root.to_string_lossy().as_ref(),
            "diff",
            "--name-only",
            "HEAD",
        ])
        .output();
    let Ok(output) = output else {
        return "unknown".to_string();
    };
    if !output.status.success() {
        return "unknown".to_string();
    }
    let mut languages = BTreeSet::new();
    for path in String::from_utf8_lossy(&output.stdout).lines() {
        let extension = Path::new(path)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let name = match extension.as_str() {
            "rs" => "rust",
            "js" | "mjs" | "cjs" => "javascript",
            "ts" | "tsx" => "typescript",
            "py" => "python",
            "go" => "go",
            "java" => "java",
            "rb" => "ruby",
            "cs" => "csharp",
            "cpp" | "cc" | "cxx" | "h" | "hpp" => "cpp",
            _ => continue,
        };
        languages.insert(name);
    }
    if languages.is_empty() {
        "unknown".to_string()
    } else {
        languages.into_iter().collect::<Vec<_>>().join("+")
    }
}

pub(crate) fn freshness(packet: &NextPassPacket) -> String {
    if packet.trust_assessments.is_empty() {
        return "unknown".to_string();
    }
    for state in ["stale", "environment_mismatch", "unbound", "unknown"] {
        if packet
            .trust_assessments
            .iter()
            .any(|assessment| assessment.applicability == state)
        {
            return state.to_string();
        }
    }
    "current".to_string()
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

fn eligibility(result: &str, grade: &str, model_resolved: bool) -> (&'static str, &'static str) {
    if result == "unknown" {
        return (
            "excluded",
            "unknown adjudication has no success or failure outcome",
        );
    }
    if matches!(
        grade,
        "independent-hidden-tests"
            | "benchmark-adjudication"
            | "signed-human-review"
            | "equivalent-independent"
    ) {
        if model_resolved {
            (
                "included",
                "independent cited outcome with exact model snapshot",
            )
        } else {
            (
                "excluded",
                "mutable model alias was not resolved to an exact snapshot",
            )
        }
    } else if matches!(grade, "merge" | "revert" | "incident-free-window") {
        (
            "supporting",
            "operational signal without independent task adjudication",
        )
    } else {
        (
            "excluded",
            "self-report, bare gate/label success, or model-card claim",
        )
    }
}

pub fn adjudicate(
    run_dir: &Path,
    result: &str,
    grade: &str,
    cites: &[String],
    observed_at: &str,
) -> Result<SignedJournalRecord, Box<dyn std::error::Error>> {
    if !matches!(result, "success" | "failure" | "unknown") {
        return Err("adjudication result must be success, failure, or unknown".into());
    }
    compile_run_dir(run_dir)?;
    let manifest: RunManifest = read_json(&run_dir.join("manifest.json"))?;
    let packet: NextPassPacket = read_json(&run_dir.join("state/next_pass_packet.json"))?;
    let session = latest_session(run_dir)?;
    let model = session_model(session);
    let repo_root = PathBuf::from(
        manifest
            .repo_root
            .as_deref()
            .ok_or("adjudication requires repo_root in manifest.json")?,
    );
    let citations: Vec<OutcomeCitation> = cites
        .iter()
        .map(|cite| verify_citation(cite, &repo_root, observed_at))
        .collect::<Result<_, _>>()?;
    if citations.is_empty() {
        return Err("adjudication requires at least one independently verifiable citation".into());
    }
    let (training_eligibility, eligibility_reason) =
        eligibility(result, grade, model.model_specific_eligible);
    let executor = current_executor_identity()?;
    let attempts = load_verified_attempts(run_dir)?;
    let negative_controls = attempts
        .iter()
        .filter(|attempt| attempt.negative_control_outcome.as_deref() == Some("expected_failure"))
        .count();
    let environment_match = if packet.trust_assessments.is_empty() {
        "unknown"
    } else if packet
        .trust_assessments
        .iter()
        .any(|assessment| assessment.applicability == "environment_mismatch")
    {
        "mismatch"
    } else {
        "matched"
    };
    let mut claim_ids: Vec<String> = packet
        .trust_assessments
        .iter()
        .map(|assessment| assessment.subject_id.clone())
        .collect();
    claim_ids.sort();
    claim_ids.dedup();
    let outcome = IndependentOutcome {
        outcome_id: format!(
            "outcome-{}",
            &blake3::hash(
                format!("{}:{result}:{grade}:{observed_at}", manifest.objective_id).as_bytes()
            )
            .to_hex()[..16]
        ),
        task_id: manifest.objective_id,
        claim_ids,
        result: result.to_string(),
        source_grade: grade.to_string(),
        training_eligibility: training_eligibility.to_string(),
        eligibility_reason: eligibility_reason.to_string(),
        observation_window_start: manifest.created_at,
        observation_window_end: observed_at.to_string(),
        adjudicator: AdjudicatorIdentity {
            principal_id: format!("human:{}", executor.key_fingerprint),
            authenticated_by: executor.principal_id,
            role: if grade == "signed-human-review" {
                "human-review".to_string()
            } else {
                "independent-adjudicator".to_string()
            },
            independent_from_worker: training_eligibility == "included",
        },
        citations,
        model,
        task_family: task_family(&packet),
        repository_id: repository_id(&repo_root),
        language: language(&repo_root),
        freshness: freshness(&packet),
        change_size: change_size(&repo_root),
        check_strength: format!(
            "{} bound check history item(s), {} expected-failure negative control(s)",
            packet.check_histories.len(),
            negative_controls
        ),
        environment_match: environment_match.to_string(),
        retry_history: packet.check_histories,
    };
    append_signed_record(run_dir, JOURNAL, KIND, serde_json::to_value(outcome)?)
}

pub fn load_outcome_records(
    run_dir: &Path,
) -> Result<Vec<(IndependentOutcome, String)>, Box<dyn std::error::Error>> {
    load_verified_signed_records(run_dir, JOURNAL, KIND)?
        .into_iter()
        .map(|record| {
            let outcome = serde_json::from_value(record.payload)?;
            Ok((outcome, record.record_hash))
        })
        .collect()
}
