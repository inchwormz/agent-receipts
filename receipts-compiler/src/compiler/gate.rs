//! Categorical, fail-closed release gate.
//!
//! Cryptographic and source-integrity validation belongs to `compile_run_dir`.
//! This gate recompiles first, then consumes the engine-authored packet for
//! completion policy. No statistical score can override a red category.

use crate::compiler::run_dir::{RunManifest, compile_run_dir};
use crate::schema::{EvidenceRecord, NextPassPacket, VerifierFinding};
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct GateFindingStatus {
    pub id: String,
    pub status: String,
    pub verifier_score: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct StrictGateReport {
    pub ok: bool,
    pub run_dir: String,
    pub pass_id: String,
    pub packet_pass_id: String,
    pub evidence_count: usize,
    pub verifier_findings: Vec<GateFindingStatus>,
    pub halt_kinds: Vec<String>,
    pub candidate_actions: usize,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn read_jsonl<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Vec<T>, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    fs::read_to_string(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    Ok(files)
}

fn semantic_kind_requires_file(kind: &str) -> bool {
    matches!(kind, "code-change" | "root-cause" | "test-change")
}

pub fn run_gate(run_dir: &Path) -> Result<StrictGateReport, Box<dyn std::error::Error>> {
    compile_run_dir(run_dir)?;
    let manifest: RunManifest = read_json(&run_dir.join("manifest.json"))?;
    let packet: NextPassPacket = read_json(&run_dir.join("state/next_pass_packet.json"))?;
    let evidence: Vec<EvidenceRecord> = read_jsonl(&run_dir.join("worker-results/evidence.jsonl"))?;
    let findings: Vec<VerifierFinding> =
        read_jsonl(&run_dir.join("verifier-results/findings.jsonl"))?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if manifest.pass_id != packet.pass_id {
        errors.push(format!(
            "manifest pass_id {} does not match packet pass_id {}",
            manifest.pass_id, packet.pass_id
        ));
    }
    if manifest.pass_id == "pass-0001" {
        errors.push(
            "run is still pass-0001; strict gate requires at least one promoted recurrence pass"
                .to_string(),
        );
    }
    if evidence.len() <= 1 {
        errors.push(
            "run has only objective evidence; subagent/compiler promotion has not happened"
                .to_string(),
        );
    }
    if let Some(record) = evidence.iter().find(|record| record.source_ids.is_empty()) {
        errors.push(format!("evidence record {} lacks source_ids", record.id));
    }
    if let Some(record) = findings.iter().find(|record| record.source_ids.is_empty()) {
        errors.push(format!("verifier finding {} lacks source_ids", record.id));
    }

    let missing_semantic_files: Vec<&str> = evidence
        .iter()
        .filter(|record| semantic_kind_requires_file(&record.kind))
        .filter(|record| {
            !record.source_refs.iter().any(|source| {
                source.kind == "file"
                    && source.hash_basis.as_deref() == Some("content")
                    && matches!(source.hash_alg.as_str(), "fnv1a-64" | "blake3-256")
            })
        })
        .map(|record| record.id.as_str())
        .collect();
    if !missing_semantic_files.is_empty() {
        errors.push(format!(
            "evidence records of kind code-change/root-cause/test-change lack a content-hashed file source_ref: {}",
            missing_semantic_files.join(", ")
        ));
    }

    let raw_subagent_files = walk_files(&run_dir.join("raw/subagents"))?;
    let sessions: Vec<&EvidenceRecord> = evidence
        .iter()
        .filter(|record| record.kind == "subagent-session")
        .collect();
    if raw_subagent_files.is_empty() {
        errors.push(
            "no raw/subagents session artifacts found; subagent outputs were not quarantined"
                .to_string(),
        );
    }
    if sessions.is_empty() {
        errors.push("no subagent-session evidence records found; raw subagent output was not ingested mechanically".to_string());
    }
    let compiled_sources: BTreeSet<&str> = packet
        .sources
        .iter()
        .map(|source| source.source_id.as_str())
        .collect();
    if !raw_subagent_files.is_empty()
        && !compiled_sources
            .iter()
            .any(|source_id| source_id.starts_with("raw:subagents/"))
    {
        errors.push(
            "raw/subagents artifacts were not compiled into next_pass_packet.sources".to_string(),
        );
    }
    for session in sessions {
        if session.agent_id.as_deref().unwrap_or("").trim().is_empty()
            || session.lane.as_deref().unwrap_or("").trim().is_empty()
        {
            errors.push(format!(
                "subagent-session evidence {} must carry non-empty agent_id and lane",
                session.id
            ));
        }
        if !session
            .source_ids
            .iter()
            .any(|source_id| source_id.starts_with("raw:subagents/"))
        {
            errors.push(format!(
                "subagent-session evidence {} does not reference raw:subagents/*",
                session.id
            ));
        }
    }

    let min_agents = std::env::var("RECEIPTS_MIN_AGENT_COVERAGE")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(3);
    let agents: BTreeSet<&str> = evidence
        .iter()
        .filter(|record| record.id != "ev-objective")
        .filter(|record| !record.id.starts_with("ev-subagent-session-"))
        .filter(|record| !matches!(record.kind.as_str(), "unstructured" | "receipt" | "work"))
        .filter_map(|record| record.agent_id.as_deref())
        .filter(|agent| !agent.trim().is_empty())
        .collect();
    let readiness_escape = packet.pass_id == "pass-0001"
        && findings.len() == 1
        && findings[0].id == "vf-codex-synthesis-pending";
    if !readiness_escape && agents.len() < min_agents {
        errors.push(format!(
            "agent-id coverage floor not met: saw {} distinct agent_id(s), need at least {min_agents}",
            agents.len()
        ));
    }

    let has_synthesis_evidence = evidence
        .iter()
        .any(|record| record.kind == "codex-synthesis");
    let has_synthesis_raw = walk_files(&run_dir.join("raw"))?.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("codex-synthesis-"))
    });
    if !has_synthesis_evidence || !has_synthesis_raw {
        errors.push("Codex synthesis was not recorded through receipts synthesize".to_string());
    }
    let non_passing: Vec<&str> = findings
        .iter()
        .filter(|finding| finding.status != "passed")
        .map(|finding| finding.id.as_str())
        .collect();
    if !non_passing.is_empty() {
        errors.push(format!(
            "non-passing verifier findings remain: {}",
            non_passing.join(", ")
        ));
    }
    for contradiction in &packet.contradictions {
        if contradiction.id.starts_with("con:receipt:") {
            errors.push(format!(
                "refuted by execution receipt: {}",
                contradiction.summary
            ));
        }
    }
    let halt_kinds: Vec<String> = packet
        .halt_signals
        .iter()
        .map(|signal| signal.kind.clone())
        .collect();
    if !halt_kinds.iter().any(|kind| kind == "ready-to-halt") {
        errors.push("packet is not ready-to-halt".to_string());
    }
    for item in &packet.candidate_actions {
        if item.blocking == Some(true) && item.resolved != Some(true) {
            errors.push(format!(
                "unresolved blocking worklist item [{}] {}: {}",
                item.category.as_deref().unwrap_or("?"),
                item.id,
                item.title
            ));
        }
    }
    if packet.sources.len() < evidence.len() + findings.len() {
        warnings.push(
            "packet source count is lower than evidence+finding count; inspect source promotion"
                .to_string(),
        );
    }

    let report = StrictGateReport {
        ok: errors.is_empty(),
        run_dir: run_dir.display().to_string(),
        pass_id: manifest.pass_id,
        packet_pass_id: packet.pass_id,
        evidence_count: evidence.len(),
        verifier_findings: findings
            .into_iter()
            .map(|finding| GateFindingStatus {
                id: finding.id,
                status: finding.status,
                verifier_score: finding.verifier_score,
            })
            .collect(),
        halt_kinds,
        candidate_actions: packet.candidate_actions.len(),
        errors,
        warnings,
    };
    let state_dir = run_dir.join("state");
    fs::create_dir_all(&state_dir)?;
    let mut bytes = serde_json::to_vec_pretty(&report)?;
    bytes.push(b'\n');
    fs::write(state_dir.join("gate-report.json"), bytes)?;
    Ok(report)
}
