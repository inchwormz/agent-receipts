use crate::compiler::artifacts::ArtifactRef;
use crate::compiler::checks::{
    CheckAttemptRecord, ClaimBinding, build_check_histories, claim_binding, load_verified_attempts,
    verify_attempt_receipts,
};
use crate::compiler::contradictions::detect_auto_contradictions;
use crate::compiler::journal::append_decision_log;
use crate::compiler::packets::{CompilerInputBundle, build_next_pass_packet};
use crate::compiler::receipts::load_verified_receipts;
use crate::compiler::resolutions::{ResolutionRecord, load_verified_resolutions};
use crate::compiler::signals::detect_recurring_failure_patterns;
use crate::compiler::snapshot::build_snapshot;
use crate::schema::{
    CandidateAction, CompiledFact, Contradiction, DecisionLogRecord, EvidenceCoverage,
    EvidenceRecord, HaltSignal, Hypothesis, RECEIPTS_HASH_ALG, ReceiptEvent, ReceiptRecord,
    SnapshotInput, SourceRef, StateDelta, TrustAssessment, VerifierFinding, WorkerResult,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunManifest {
    pub objective_id: String,
    pub run_id: String,
    pub branch_id: String,
    pub pass_id: String,
    pub created_at: String,
    /// F3: the project root that `file:` citations resolve against. Recorded
    /// at run creation; without it, file refs outside the run dir cannot be
    /// verified (there is deliberately no fallback to any build-time or
    /// package-relative root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunDirCompileReport {
    pub snapshot_path: PathBuf,
    pub packet_path: PathBuf,
    pub decision_log_path: PathBuf,
    pub evidence_count: usize,
    pub verifier_finding_count: usize,
}

pub fn compile_run_dir(run_dir: &Path) -> Result<RunDirCompileReport, Box<dyn std::error::Error>> {
    let manifest = read_json::<RunManifest>(&run_dir.join("manifest.json"))?;
    let objective = fs::read_to_string(run_dir.join("task.md"))?
        .trim()
        .to_string();
    let raw_sources = load_sources(&run_dir.join("raw"), &manifest.created_at)?;
    let mut worker_evidence =
        read_jsonl::<EvidenceRecord>(&run_dir.join("worker-results/evidence.jsonl"))?;
    let mut verifier_findings =
        read_jsonl::<VerifierFinding>(&run_dir.join("verifier-results/findings.jsonl"))?;

    let repo_root = manifest.repo_root.as_deref();
    verify_declared_file_refs(run_dir, repo_root, &worker_evidence, &verifier_findings)?;
    version_temporal_file_sources(&mut worker_evidence, &mut verifier_findings)?;

    // M1/M2: load the execution-receipt journal. A broken hash chain is a
    // hard compile error - receipts are the one artifact whose integrity is
    // non-negotiable. Each receipt becomes a packet source plus a
    // runtime-authored evidence record, and EXEC receipt outcomes attest or
    // refute claims below. WORK receipts (label work:tree) attest tree state
    // only - they are partitioned out of every claim-attestation path.
    let receipts = load_verified_receipts(run_dir)?;
    let check_attempts = load_verified_attempts(run_dir)?;
    verify_attempt_receipts(&check_attempts, &receipts)?;
    let check_histories = build_check_histories(&check_attempts);
    let (_work_receipts, exec_receipts): (Vec<&ReceiptRecord>, Vec<&ReceiptRecord>) =
        receipts.iter().partition(|receipt| {
            receipt.label.as_deref() == Some(crate::compiler::receipts::WORK_LABEL)
        });
    let exec_receipts: Vec<ReceiptRecord> = exec_receipts.into_iter().cloned().collect();
    let receipt_events = build_receipt_events(&receipts, &check_attempts);

    let mut sources = raw_sources.clone();
    sources.extend(receipts.iter().map(receipt_source_ref));
    sources.extend(evidence_declared_sources(&worker_evidence));
    sources.extend(verifier_declared_sources(&verifier_findings));
    sources.extend(evidence_sources(&worker_evidence));
    sources.extend(verifier_sources(&verifier_findings, &manifest.created_at));
    dedupe_sources_strict(&mut sources)?;

    let artifact_refs: Vec<ArtifactRef> = sources.clone();
    let snapshot = build_snapshot(
        manifest.run_id.clone(),
        manifest.pass_id.clone(),
        manifest.branch_id.clone(),
        manifest.created_at.clone(),
        vec![SnapshotInput {
            id: "input:task".to_string(),
            kind: "task".to_string(),
            summary: objective.clone(),
            ref_ids: raw_sources
                .first()
                .map(|source| vec![source.source_id.clone()])
                .unwrap_or_default(),
        }],
        worker_evidence
            .iter()
            .map(|evidence| WorkerResult {
                id: format!("worker:{}", evidence.id),
                worker: "local-evidence-ingest".to_string(),
                status: "ok".to_string(),
                output_ids: evidence.source_ids.clone(),
                notes: evidence.summary.clone(),
            })
            .collect(),
        state_delta_from_evidence(&worker_evidence),
        artifact_refs,
    )?;

    let recurring_patterns =
        detect_recurring_failure_patterns(&verifier_findings, &manifest.created_at);
    // Auto-detect same-span divergent-summary contradictions between evidence
    // records. This surfaces disagreement between workers that cite the same
    // direct source (file/command/test) with differing summaries or attribution
    // so Prime can see it in the recompiled packet.
    let mut auto_contradictions = detect_auto_contradictions(&worker_evidence);
    // M2: a passed verifier finding whose cited label has a FAILING latest
    // receipt is refuted mechanically - the strongest signal this system
    // produces, and it requires zero agent cooperation.
    auto_contradictions.extend(detect_receipt_refutations(
        &verifier_findings,
        &exec_receipts,
    ));
    auto_contradictions.sort_by(|a, b| a.id.cmp(&b.id));
    // F1 + M2: trusted_facts are GATED, not relabeled evidence. Receipt-backed
    // records promote as "attested"; verifier-backed as "verifier"; everything
    // else stays in the evidence section. Only EXEC receipts participate.
    let repo_root_path = manifest.repo_root.as_deref().map(Path::new);
    let claim_bindings: HashMap<String, ClaimBinding> = worker_evidence
        .iter()
        .map(|record| {
            Ok((
                record.id.clone(),
                claim_binding(repo_root_path, &record.id, &record.kind, &check_attempts)?,
            ))
        })
        .collect::<Result<_, Box<dyn std::error::Error>>>()?;
    let trusted_facts = facts_from_evidence(
        &worker_evidence,
        &verifier_findings,
        &auto_contradictions,
        &claim_bindings,
    );

    let trusted_facts_for_digests = trusted_facts.clone();
    let trust_assessments = build_trust_assessments(
        &worker_evidence,
        &trusted_facts,
        &auto_contradictions,
        &claim_bindings,
    );
    let evidence_coverage = compute_evidence_coverage(&worker_evidence, &trust_assessments);

    // Phase 2: the derived worklist replaces template candidate_actions.
    // Resolutions (Prime's typed adjudications, hash-chained) clear blocking
    // items; a broken resolution chain is a hard compile error.
    let resolutions = load_verified_resolutions(run_dir)?;
    let worklist = derive_worklist(
        &worker_evidence,
        &verifier_findings,
        &auto_contradictions,
        &trusted_facts,
        &resolutions,
    );
    let packet = build_next_pass_packet(CompilerInputBundle {
        objective_id: manifest.objective_id.clone(),
        run_id: manifest.run_id.clone(),
        branch_id: manifest.branch_id.clone(),
        pass_id: manifest.pass_id.clone(),
        objective,
        evidence: worker_evidence.clone(),
        trusted_facts,
        active_hypotheses: vec![],
        contradictions: auto_contradictions.clone(),
        recurring_failure_patterns: recurring_patterns,
        candidate_actions: worklist.clone(),
        verifier_findings: verifier_findings.clone(),
        open_questions: open_questions_from_failures(&verifier_findings),
        raw_drilldown_refs: raw_sources,
        halt_signals: halt_signals_from_state(&verifier_findings, &worklist, &manifest.created_at),
        sources,
        lane_digests: derive_lane_digests(
            &worker_evidence,
            &trusted_facts_for_digests,
            &auto_contradictions,
            &exec_receipts,
        ),
        trust_assessments,
        receipt_events,
        evidence_coverage,
        check_histories,
    })?;

    let state_dir = run_dir.join("state");
    fs::create_dir_all(&state_dir)?;
    let snapshot_path = state_dir.join("snapshot.json");
    let packet_path = state_dir.join("next_pass_packet.json");
    let decision_log_path = state_dir.join("decision_log.jsonl");

    write_json(&snapshot_path, &snapshot)?;
    write_json(&packet_path, &packet)?;
    append_decision_log(
        &decision_log_path,
        &DecisionLogRecord {
            id: format!("decision:{}:{}", manifest.run_id, manifest.pass_id),
            run_id: manifest.run_id,
            pass_id: manifest.pass_id,
            decision_kind: "compile-next-pass-packet".to_string(),
            summary: "Compiled raw run artifacts into a source-backed next-pass packet."
                .to_string(),
            source_ids: packet
                .sources
                .iter()
                .map(|source| source.source_id.clone())
                .collect(),
            selected_action_ids: packet
                .candidate_actions
                .iter()
                .map(|action| action.id.clone())
                .collect(),
            created_at: manifest.created_at,
            promotion: None,
        },
    )?;

    // F12: the compiler is the single writer of the input fingerprint. The
    // strict gate refuses to run without it (no mtime fallback), so staleness
    // is always judged against content hashes of the actual inputs.
    write_input_fingerprint(run_dir, &state_dir)?;

    Ok(RunDirCompileReport {
        snapshot_path,
        packet_path,
        decision_log_path,
        evidence_count: packet.evidence.len(),
        verifier_finding_count: packet.verifier_findings.len(),
    })
}

#[derive(Serialize)]
struct FingerprintEntry {
    path: String,
    size: u64,
    hash: String,
}

/// Mirrors the JS `inputFingerprint` shape exactly: manifest.json, task.md,
/// and every file under the input journals (including checks/), sorted by
/// absolute path string, each entry carrying run-dir-relative forward-slash
/// path, byte size, and fnv1a-64 content hash.
fn write_input_fingerprint(
    run_dir: &Path,
    state_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut files: Vec<PathBuf> = Vec::new();
    for name in ["manifest.json", "task.md"] {
        let candidate = run_dir.join(name);
        if candidate.exists() {
            files.push(candidate);
        }
    }
    for dir in [
        "raw",
        "worker-results",
        "verifier-results",
        "receipts",
        "decisions",
        "checks",
    ] {
        collect_files_recursive(&run_dir.join(dir), &mut files)?;
    }
    files.sort_by_key(|path| path.to_string_lossy().to_string());

    let mut entries: Vec<FingerprintEntry> = Vec::new();
    for file in files {
        let bytes = fs::read(&file)?;
        entries.push(FingerprintEntry {
            path: file
                .strip_prefix(run_dir)?
                .to_string_lossy()
                .replace('\\', "/"),
            size: bytes.len() as u64,
            hash: fnv1a_hash(&bytes),
        });
    }
    write_json(&state_dir.join("input_fingerprint.json"), &entries)
}

fn collect_files_recursive(
    dir: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn read_json<T>(path: &Path) -> Result<T, Box<dyn std::error::Error>>
where
    T: for<'de> Deserialize<'de>,
{
    let file = File::open(path)?;
    Ok(serde_json::from_reader(file)?)
}

fn write_json<T>(path: &Path, value: &T) -> Result<(), Box<dyn std::error::Error>>
where
    T: Serialize,
{
    let mut file = File::create(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn read_jsonl<T>(path: &Path) -> Result<Vec<T>, Box<dyn std::error::Error>>
where
    T: for<'de> Deserialize<'de>,
{
    if !path.exists() {
        return Ok(vec![]);
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut values = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        values.push(serde_json::from_str(&line)?);
    }

    Ok(values)
}

fn load_sources(
    raw_dir: &Path,
    observed_at: &str,
) -> Result<Vec<SourceRef>, Box<dyn std::error::Error>> {
    if !raw_dir.exists() {
        return Ok(vec![]);
    }

    let mut sources = Vec::new();
    collect_raw_sources(raw_dir, raw_dir, observed_at, &mut sources)?;
    sources.sort_by(|left, right| left.source_id.cmp(&right.source_id));
    Ok(sources)
}

fn collect_raw_sources(
    raw_dir: &Path,
    current_dir: &Path,
    observed_at: &str,
    sources: &mut Vec<SourceRef>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Sort entries so traversal order is stable across file systems.
    let mut entries: Vec<_> = fs::read_dir(current_dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_raw_sources(raw_dir, &path, observed_at, sources)?;
            continue;
        }
        if path.is_file() {
            let bytes = fs::read(&path)?;
            let source_path = path
                .strip_prefix(raw_dir)?
                .to_string_lossy()
                .replace('\\', "/")
                .to_string();
            sources.push(SourceRef {
                source_id: format!("raw:{source_path}"),
                path: format!("raw/{source_path}"),
                kind: "raw".to_string(),
                hash: fnv1a_hash(&bytes),
                hash_alg: RECEIPTS_HASH_ALG.to_string(),
                hash_basis: Some("content".to_string()),
                span: None,
                observed_at: observed_at.to_string(),
            });
        }
    }
    Ok(())
}

fn evidence_sources(evidence: &[EvidenceRecord]) -> Vec<SourceRef> {
    evidence
        .iter()
        .map(|item| SourceRef {
            source_id: format!("evidence:{}", item.id),
            path: "worker-results/evidence.jsonl".to_string(),
            kind: item.kind.clone(),
            hash: fnv1a_hash(item.summary.as_bytes()),
            hash_alg: RECEIPTS_HASH_ALG.to_string(),
            hash_basis: None,
            span: Some(item.id.clone()),
            observed_at: item.observed_at.clone(),
        })
        .collect()
}

fn evidence_declared_sources(evidence: &[EvidenceRecord]) -> Vec<SourceRef> {
    evidence
        .iter()
        .flat_map(|item| item.source_refs.clone())
        .collect()
}

fn verifier_sources(findings: &[VerifierFinding], observed_at: &str) -> Vec<SourceRef> {
    findings
        .iter()
        .map(|finding| SourceRef {
            source_id: format!("verifier:{}", finding.id),
            path: "verifier-results/findings.jsonl".to_string(),
            kind: "verifier".to_string(),
            hash: fnv1a_hash(finding.summary.as_bytes()),
            hash_alg: RECEIPTS_HASH_ALG.to_string(),
            hash_basis: None,
            span: Some(finding.id.clone()),
            observed_at: observed_at.to_string(),
        })
        .collect()
}

fn verifier_declared_sources(findings: &[VerifierFinding]) -> Vec<SourceRef> {
    findings
        .iter()
        .flat_map(|finding| finding.source_refs.clone())
        .collect()
}

/// A repo file citation identifies bytes observed at a moment, not one
/// immutable path for the lifetime of a run. When that path has multiple
/// observed hashes, make the content pin part of the source id and update the
/// owning record atomically. Other source kinds keep strict global identity.
fn version_temporal_file_sources(
    evidence: &mut [EvidenceRecord],
    findings: &mut [VerifierFinding],
) -> Result<(), Box<dyn std::error::Error>> {
    let mut variants: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for record in evidence.iter() {
        collect_record_file_variants("evidence", &record.id, &record.source_refs, &mut variants)?;
    }
    for finding in findings.iter() {
        collect_record_file_variants(
            "verifier_finding",
            &finding.id,
            &finding.source_refs,
            &mut variants,
        )?;
    }

    let temporal_ids: BTreeSet<String> = variants
        .into_iter()
        .filter_map(|(source_id, variants)| (variants.len() > 1).then_some(source_id))
        .collect();
    if temporal_ids.is_empty() {
        return Ok(());
    }

    for record in evidence.iter_mut() {
        version_record_file_sources(
            &mut record.source_ids,
            &mut record.source_refs,
            &temporal_ids,
        );
    }
    for finding in findings.iter_mut() {
        version_record_file_sources(
            &mut finding.source_ids,
            &mut finding.source_refs,
            &temporal_ids,
        );
    }
    Ok(())
}

fn collect_record_file_variants(
    label: &str,
    record_id: &str,
    refs: &[SourceRef],
    variants: &mut BTreeMap<String, BTreeSet<(String, String)>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut local: HashMap<&str, (&str, &str)> = HashMap::new();
    for source in refs
        .iter()
        .filter(|source| source.kind == "file" && source.source_id.starts_with("file:"))
    {
        let variant = (source.hash_alg.as_str(), source.hash.as_str());
        if let Some(prior) = local.insert(source.source_id.as_str(), variant)
            && prior != variant
        {
            return Err(format!(
                "{label} `{record_id}` declares file source_id `{}` twice with divergent hash",
                source.source_id
            )
            .into());
        }
        variants
            .entry(source.source_id.clone())
            .or_default()
            .insert((source.hash_alg.clone(), source.hash.clone()));
    }
    Ok(())
}

fn version_record_file_sources(
    source_ids: &mut [String],
    source_refs: &mut [SourceRef],
    temporal_ids: &BTreeSet<String>,
) {
    let mut replacements: HashMap<String, String> = HashMap::new();
    for source in source_refs.iter_mut() {
        if source.kind != "file" || !temporal_ids.contains(&source.source_id) {
            continue;
        }
        let original = source.source_id.clone();
        let versioned = format!("{original}@{}:{}", source.hash_alg, source.hash);
        source.source_id = versioned.clone();
        replacements.insert(original, versioned);
    }
    for source_id in source_ids.iter_mut() {
        if let Some(versioned) = replacements.get(source_id) {
            *source_id = versioned.clone();
        }
    }
}

/// Dedupe packet sources by `source_id`, and fail hard when two refs share an
/// id but disagree on content-hash or hash algorithm. Path and observed_at are
/// treated as non-canonical (tools may report absolute vs relative paths or
/// different ingest timestamps). The first-seen record wins.
fn dedupe_sources_strict(sources: &mut Vec<SourceRef>) -> Result<(), Box<dyn std::error::Error>> {
    sources.sort_by(|left, right| left.source_id.cmp(&right.source_id));

    let mut seen: HashMap<String, SourceRef> = HashMap::new();
    for source in sources.iter() {
        if let Some(prior) = seen.get(&source.source_id) {
            if prior.hash != source.hash || prior.hash_alg != source.hash_alg {
                return Err(format!(
                    "source_id `{}` declared twice with divergent hash: `{}` ({}) vs `{}` ({})",
                    source.source_id, prior.hash, prior.hash_alg, source.hash, source.hash_alg,
                )
                .into());
            }
            continue;
        }
        seen.insert(source.source_id.clone(), source.clone());
    }
    sources.dedup_by(|left, right| left.source_id == right.source_id);
    Ok(())
}

fn verify_declared_file_refs(
    run_dir: &Path,
    repo_root: Option<&str>,
    evidence: &[EvidenceRecord],
    findings: &[VerifierFinding],
) -> Result<(), Box<dyn std::error::Error>> {
    for record in evidence {
        verify_record_file_refs(
            run_dir,
            repo_root,
            "evidence",
            &record.id,
            &record.source_refs,
        )?;
    }
    for finding in findings {
        verify_record_file_refs(
            run_dir,
            repo_root,
            "verifier_finding",
            &finding.id,
            &finding.source_refs,
        )?;
    }
    Ok(())
}

fn verify_record_file_refs(
    run_dir: &Path,
    repo_root: Option<&str>,
    label: &str,
    id: &str,
    refs: &[SourceRef],
) -> Result<(), Box<dyn std::error::Error>> {
    for source in refs {
        if source.hash_alg != RECEIPTS_HASH_ALG {
            return Err(format!(
                "{label} `{id}` source_ref `{}` uses unsupported hash_alg `{}` (expected `{RECEIPTS_HASH_ALG}`)",
                source.source_id, source.hash_alg
            )
            .into());
        }
        if source.kind != "file" {
            continue;
        }
        let resolved = resolve_source_path(run_dir, repo_root, &source.path);
        let bytes = fs::read(&resolved).map_err(|err| {
            format!(
                "{label} `{id}` source_ref `{}` file path not readable: {} ({err}). \
                 File refs resolve against the run dir, then manifest.repo_root — \
                 if this citation targets the project tree, ensure repo_root is set in manifest.json",
                source.source_id,
                resolved.display()
            )
        })?;
        let actual = fnv1a_hash(&bytes);
        if source.hash != actual {
            // CUSTODY vs DRIFT (field lesson, 2026-07-13 dogfood): artifacts
            // INSIDE the run dir are quarantined - a mismatch there is
            // tampering and fails hard. Refs into the PROJECT TREE legitimately
            // drift when Prime applies post-review fixes; the ingest-time hash
            // already pins what the agent actually saw, so drift is the gate's
            // warning, not a compile failure.
            let inside_run = resolved
                .canonicalize()
                .ok()
                .zip(run_dir.canonicalize().ok())
                .is_some_and(|(res, run)| res.starts_with(&run));
            if inside_run {
                return Err(format!(
                    "{label} `{id}` source_ref `{}` hash mismatch inside the run dir: expected `{actual}`, got `{}` - quarantined artifacts are immutable",
                    source.source_id, source.hash
                )
                .into());
            }
            // Drifted repo citation: skip span validation too (the file's
            // line count may have changed since observation).
            continue;
        }
        verify_line_span(&source.span, &bytes, label, id, &source.source_id)?;
    }
    Ok(())
}

fn verify_line_span(
    span: &Option<String>,
    bytes: &[u8],
    label: &str,
    id: &str,
    source_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(span) = span else {
        return Ok(());
    };
    let line_count = std::str::from_utf8(bytes)
        .map(|text| text.lines().count().max(1))
        .unwrap_or(1);
    let (start_str, end_str) = match span.split_once('-') {
        Some((a, b)) => (a, b),
        None => (span.as_str(), span.as_str()),
    };
    let start: usize = start_str.parse().map_err(|_| {
        format!("{label} `{id}` source_ref `{source_id}` span `{span}` is not a line range")
    })?;
    let end: usize = end_str.parse().map_err(|_| {
        format!("{label} `{id}` source_ref `{source_id}` span `{span}` is not a line range")
    })?;
    if start < 1 || end < start || end > line_count {
        return Err(format!(
            "{label} `{id}` source_ref `{source_id}` span `{span}` is outside file line range 1-{line_count}"
        )
        .into());
    }
    Ok(())
}

fn resolve_source_path(run_dir: &Path, repo_root: Option<&str>, source_path: &str) -> PathBuf {
    let candidate = PathBuf::from(source_path);
    if candidate.is_absolute() {
        return candidate;
    }

    // Prefer paths inside run_dir (raw/, worker-results/, verifier-results/).
    let inside_run = run_dir.join(&candidate);
    if inside_run.exists() {
        return inside_run;
    }

    // F3: project-tree refs resolve ONLY against the manifest's recorded
    // repo_root. There is deliberately no build-time or package-relative
    // fallback — that class of fallback made citations verify against the
    // wrong repository.
    if let Some(root) = repo_root {
        let candidate_abs = Path::new(root).join(&candidate);
        if candidate_abs.exists() {
            return candidate_abs;
        }
    }
    candidate
}

/// Evidence kinds that are infrastructure or quarantined prose, never
/// candidate facts.
const NON_FACT_KINDS: &[&str] = &[
    "objective",
    "subagent-session",
    "codex-synthesis",
    "unstructured",
    "blocker",
];

/// One packet source per receipt: content-hashed (the record_hash covers the
/// receipt's full content), span = receipt id inside the journal.
fn receipt_source_ref(receipt: &ReceiptRecord) -> SourceRef {
    SourceRef {
        source_id: format!("receipt:{}", receipt.id),
        path: "receipts/receipts.jsonl".to_string(),
        kind: "receipt".to_string(),
        hash: receipt.record_hash.clone(),
        hash_alg: RECEIPTS_HASH_ALG.to_string(),
        hash_basis: Some("content".to_string()),
        span: Some(receipt.id.clone()),
        observed_at: receipt.ended_at.clone(),
    }
}

fn build_receipt_events(
    receipts: &[ReceiptRecord],
    check_attempts: &[CheckAttemptRecord],
) -> Vec<ReceiptEvent> {
    let mut attempts: HashMap<String, u32> = HashMap::new();
    let expected_failures: BTreeSet<&str> = check_attempts
        .iter()
        .filter(|attempt| attempt.negative_control_outcome.as_deref() == Some("expected_failure"))
        .filter_map(|attempt| attempt.negative_control_receipt_id.as_deref())
        .collect();
    receipts
        .iter()
        .map(|receipt| {
            let key = receipt
                .label
                .clone()
                .unwrap_or_else(|| format!("receipt:{}", receipt.id));
            let attempt = attempts.entry(key).or_insert(0);
            *attempt += 1;
            ReceiptEvent {
                receipt_id: receipt.id.clone(),
                label: receipt.label.clone(),
                integrity: "hash_verified".to_string(),
                outcome: if expected_failures.contains(receipt.id.as_str()) {
                    "expected_failure".to_string()
                } else if receipt.exit_code == 0 {
                    "passed".to_string()
                } else {
                    "failed".to_string()
                },
                exit_code: receipt.exit_code,
                attempts_for_label: *attempt,
            }
        })
        .collect()
}

/// One runtime-authored evidence record per receipt. Agents cannot write
/// these (ingest downgrades impersonations); they enter the packet straight
/// from the verified journal.
#[allow(dead_code)]
fn receipt_evidence_record(receipt: &ReceiptRecord) -> EvidenceRecord {
    let label_note = receipt
        .label
        .as_deref()
        .map(|label| format!(" [attests {label}]"))
        .unwrap_or_default();
    EvidenceRecord {
        id: format!("ev-{}", receipt.id),
        kind: "receipt".to_string(),
        summary: format!(
            "receipts run: `{}` exited {} in {}ms{}",
            receipt.cmd.join(" "),
            receipt.exit_code,
            receipt.duration_ms,
            label_note
        ),
        source_ids: vec![format!("receipt:{}", receipt.id)],
        source_refs: vec![],
        observed_at: receipt.ended_at.clone(),
        agent_id: receipt.agent_id.clone(),
        lane: receipt.lane.clone(),
        reported_confidence: None,
        rationale: None,
        diff_ref: None,
        span_before: receipt.tree_before.clone(),
        span_after: receipt.tree_after.clone(),
        claimed_agent_id: None,
        claimed_lane: None,
        provenance_warnings: vec![],
    }
}

/// One runtime-authored evidence record per WORK receipt: what changed in the
/// tree, with a mechanical window id (exec receipt ids since the previous
/// work receipt - never a lane name; review finding 6). Stats come from the
/// receipt's content-addressed artifact; the optional note is agent/Prime
/// prose and lands in `rationale` (asserted-tier annotation only).
#[allow(dead_code)]
fn work_evidence_record(
    run_dir: &Path,
    receipt: &ReceiptRecord,
    all_receipts: &[ReceiptRecord],
) -> EvidenceRecord {
    // Window: exec receipts between the previous work receipt and this one.
    let mut window: Vec<&str> = Vec::new();
    for other in all_receipts {
        if other.id == receipt.id {
            break;
        }
        if other.label.as_deref() == Some(crate::compiler::receipts::WORK_LABEL) {
            window.clear();
        } else {
            window.push(other.id.as_str());
        }
    }
    let window_text = match (window.first(), window.last()) {
        (Some(first), Some(last)) if first == last => format!("window {first}"),
        (Some(first), Some(last)) => format!("window {first}..{last}"),
        _ => "window (no exec receipts)".to_string(),
    };

    // Stats live in the content-addressed artifact (the record is frozen).
    let artifact_path = run_dir
        .join("receipts")
        .join("artifacts")
        .join(format!("{}.txt", receipt.stdout_hash));
    let (summary_stats, note) = fs::read_to_string(&artifact_path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .map(|value| {
            (
                format!(
                    "{} file(s), +{}/-{}, {} untracked{}",
                    value["total_files"].as_u64().unwrap_or(0),
                    value["total_added"].as_u64().unwrap_or(0),
                    value["total_removed"].as_u64().unwrap_or(0),
                    value["untracked"].as_u64().unwrap_or(0),
                    if value["truncated"].as_bool().unwrap_or(false) {
                        " (file list truncated)"
                    } else {
                        ""
                    }
                ),
                value["note"].as_str().map(str::to_string),
            )
        })
        .unwrap_or_else(|| (receipt.stdout_tail.clone(), None));

    EvidenceRecord {
        id: format!("ev-{}", receipt.id),
        kind: "work".to_string(),
        summary: format!("tree delta: {summary_stats} [{window_text}]"),
        source_ids: vec![format!("receipt:{}", receipt.id)],
        source_refs: vec![],
        observed_at: receipt.ended_at.clone(),
        agent_id: receipt.agent_id.clone(),
        lane: receipt.lane.clone(),
        reported_confidence: None,
        rationale: note,
        diff_ref: None,
        span_before: receipt.tree_before.clone(),
        span_after: receipt.tree_after.clone(),
        claimed_agent_id: None,
        claimed_lane: None,
        provenance_warnings: vec![],
    }
}

/// Latest receipt per label wins (retries are legitimate; the final state of
/// a check is its state).
fn latest_receipt_per_label(receipts: &[ReceiptRecord]) -> HashMap<&str, &ReceiptRecord> {
    let mut latest: HashMap<&str, &ReceiptRecord> = HashMap::new();
    for receipt in receipts {
        if let Some(label) = receipt.label.as_deref() {
            latest.insert(label, receipt);
        }
    }
    latest
}

/// M2: mechanical refutation. A PASSED verifier finding citing a label whose
/// latest receipt FAILED is contradicted by ground truth - no judgment, no
/// agent cooperation involved.
fn detect_receipt_refutations(
    findings: &[VerifierFinding],
    receipts: &[ReceiptRecord],
) -> Vec<Contradiction> {
    let latest = latest_receipt_per_label(receipts);
    let mut out = Vec::new();
    for finding in findings.iter().filter(|finding| finding.status == "passed") {
        for source_id in &finding.source_ids {
            let Some(receipt) = latest.get(source_id.as_str()) else {
                continue;
            };
            if receipt.exit_code == 0 {
                continue;
            }
            out.push(Contradiction {
                id: format!("con:receipt:{}:{}", finding.id, receipt.id),
                summary: format!(
                    "Verifier finding {} claims \"{}\" passed, but receipt {} ran `{}` and it exited {} - the claim is refuted by execution",
                    finding.id,
                    source_id,
                    receipt.id,
                    receipt.cmd.join(" "),
                    receipt.exit_code
                ),
                conflicting_item_ids: vec![finding.id.clone()],
                severity: "high".to_string(),
                source_ids: vec![
                    format!("receipt:{}", receipt.id),
                    format!("verifier:{}", finding.id),
                ],
                source_refs: None,
            });
        }
    }
    out
}

/// F1 + M2: the attestation ladder. Tier "attested": the record cites a
/// verified receipt directly, or cites a label whose latest receipt PASSED.
/// Tier "verifier": a passed verifier finding backs it (shared direct source
/// or explicit `evidence:<id>` citation). Both tiers still require: no
/// contradiction naming the record, no provenance warnings, non-infrastructure
/// kind - except runtime receipt records, which are attested by construction.
fn facts_from_evidence(
    evidence: &[EvidenceRecord],
    _findings: &[VerifierFinding],
    contradictions: &[Contradiction],
    claim_bindings: &HashMap<String, ClaimBinding>,
) -> Vec<CompiledFact> {
    let contradicted: BTreeSet<&str> = contradictions
        .iter()
        .flat_map(|item| item.conflicting_item_ids.iter().map(String::as_str))
        .collect();

    evidence
        .iter()
        .filter(|item| !NON_FACT_KINDS.contains(&item.kind.as_str()))
        .filter(|item| item.provenance_warnings.is_empty())
        .filter(|item| !contradicted.contains(item.id.as_str()))
        .filter_map(|item| {
            let attested = claim_bindings.get(&item.id).is_some_and(|binding| {
                binding.applicability == "current" && binding.outcome == "passed"
            });
            // Raw verifier findings are agent input. They can promote only
            // after a different executor principal supplies a valid signature.
            let verifier_backed = false;
            let attestation = if attested {
                "attested"
            } else if verifier_backed {
                "verifier"
            } else {
                return None;
            };
            Some(CompiledFact {
                id: format!("fact:{}", item.id),
                statement: item.summary.clone(),
                reported_confidence: item.reported_confidence.map(|value| value as f32),
                objective_relevance: 0.8,
                novelty_gain: 0.3,
                needs_raw_drilldown: false,
                source_ids: item.source_ids.clone(),
                attestation: Some(attestation.to_string()),
            })
        })
        .collect()
}

const LOAD_BEARING_CLAIM_KINDS: &[&str] = &["code-change", "test-change", "root-cause"];

fn is_coverage_claim(record: &EvidenceRecord) -> bool {
    !NON_FACT_KINDS.contains(&record.kind.as_str())
        && !(record.rationale.as_deref() == Some("harvested-from-prose")
            && !LOAD_BEARING_CLAIM_KINDS.contains(&record.kind.as_str()))
}

fn build_trust_assessments(
    evidence: &[EvidenceRecord],
    facts: &[CompiledFact],
    contradictions: &[Contradiction],
    claim_bindings: &HashMap<String, ClaimBinding>,
) -> Vec<TrustAssessment> {
    let fact_by_subject: HashMap<&str, &CompiledFact> = facts
        .iter()
        .filter_map(|fact| fact.id.strip_prefix("fact:").map(|id| (id, fact)))
        .collect();
    let refuted: BTreeSet<&str> = contradictions
        .iter()
        .flat_map(|item| item.conflicting_item_ids.iter().map(String::as_str))
        .collect();

    evidence
        .iter()
        .map(|record| {
            let fact = fact_by_subject.get(record.id.as_str()).copied();
            let binding = claim_bindings.get(&record.id);
            let claim_status = if refuted.contains(record.id.as_str()) {
                "refuted"
            } else if binding
                .is_some_and(|value| value.applicability == "current" && value.outcome == "failed")
            {
                "refuted"
            } else {
                match fact.and_then(|value| value.attestation.as_deref()) {
                    Some("attested") => "verified",
                    Some("verifier") => "verifier_backed",
                    _ => "asserted",
                }
            };
            TrustAssessment {
                subject_id: record.id.clone(),
                integrity: if record.provenance_warnings.is_empty() {
                    "hash_verified".to_string()
                } else {
                    "invalid".to_string()
                },
                outcome: binding
                    .map(|value| value.outcome.clone())
                    .unwrap_or_else(|| "unknown".to_string()),
                applicability: binding
                    .map(|value| value.applicability.clone())
                    .unwrap_or_else(|| "unbound".to_string()),
                claim_status: claim_status.to_string(),
                verifier_independent: false,
            }
        })
        .collect()
}

fn compute_evidence_coverage(
    evidence: &[EvidenceRecord],
    assessments: &[TrustAssessment],
) -> EvidenceCoverage {
    let by_subject: HashMap<&str, &TrustAssessment> = assessments
        .iter()
        .map(|assessment| (assessment.subject_id.as_str(), assessment))
        .collect();
    let mut coverage = EvidenceCoverage::default();
    for record in evidence.iter().filter(|record| is_coverage_claim(record)) {
        coverage.total_claims += 1;
        match by_subject
            .get(record.id.as_str())
            .map(|assessment| assessment.claim_status.as_str())
        {
            Some("verified") => coverage.verified_claims += 1,
            Some("verifier_backed") => coverage.verifier_backed_claims += 1,
            Some("refuted") => coverage.refuted_claims += 1,
            _ => coverage.asserted_claims += 1,
        }
    }
    coverage
}

/// Phase 3: per-lane reading guidance. Conservative rules (review finding 5):
/// `skip-verified` only when every substantive record is promoted via a
/// receipt:-id citation or verifier backing, with zero claimed-identity
/// rewrites, zero provenance warnings, and zero contradiction involvement.
/// Label-citation attestation floors at read-unverified. Any blocker =>
/// blocked; contradiction involvement => read-adjudicate.
fn derive_lane_digests(
    evidence: &[EvidenceRecord],
    trusted_facts: &[CompiledFact],
    contradictions: &[Contradiction],
    exec_receipts: &[ReceiptRecord],
) -> Vec<crate::schema::LaneDigest> {
    const INFRA: &[&str] = &[
        "objective",
        "subagent-session",
        "codex-synthesis",
        "receipt",
        "work",
    ];
    let fact_tier: HashMap<&str, &str> = trusted_facts
        .iter()
        .filter_map(|fact| {
            fact.id
                .strip_prefix("fact:")
                .zip(fact.attestation.as_deref())
        })
        .collect();
    let contradicted: BTreeSet<&str> = contradictions
        .iter()
        .flat_map(|item| item.conflicting_item_ids.iter().map(String::as_str))
        .collect();
    let exec_ids: BTreeSet<String> = exec_receipts
        .iter()
        .map(|receipt| format!("receipt:{}", receipt.id))
        .collect();

    let mut lanes: Vec<(String, Vec<&EvidenceRecord>)> = Vec::new();
    for record in evidence {
        let lane = record
            .lane
            .clone()
            .unwrap_or_else(|| "(no lane)".to_string());
        match lanes.iter_mut().find(|(name, _)| *name == lane) {
            Some(entry) => entry.1.push(record),
            None => lanes.push((lane, vec![record])),
        }
    }

    lanes
        .into_iter()
        .map(|(lane, records)| {
            let mut attested = 0u32;
            let mut verifier = 0u32;
            let mut asserted = 0u32;
            let mut warnings = 0u32;
            let mut lane_contradictions = 0u32;
            let mut blocked = false;
            let mut label_backed_attestation = false;
            let mut claimed_identity = false;
            let mut drill_down: Vec<String> = Vec::new();

            for record in &records {
                if record.kind == "blocker" {
                    blocked = true;
                }
                if !record.provenance_warnings.is_empty() {
                    warnings += 1;
                }
                if record.claimed_agent_id.is_some() || record.claimed_lane.is_some() {
                    claimed_identity = true;
                }
                if contradicted.contains(record.id.as_str()) {
                    lane_contradictions += 1;
                }
                for id in &record.source_ids {
                    if id.starts_with("raw:subagents/") && id.rfind('-').is_some() {
                        if let Some(rest) = id.rsplit(':').next() {
                            if rest.contains('-')
                                && rest.split('-').all(|part| part.parse::<u64>().is_ok())
                                && drill_down.len() < 20
                                && !drill_down.contains(id)
                            {
                                drill_down.push(id.clone());
                            }
                        }
                    }
                }
                if INFRA.contains(&record.kind.as_str()) {
                    continue;
                }
                match fact_tier.get(record.id.as_str()) {
                    Some(&"attested") => {
                        attested += 1;
                        let receipt_cited = record
                            .source_ids
                            .iter()
                            .any(|id| exec_ids.contains(id.as_str()));
                        if !receipt_cited {
                            label_backed_attestation = true;
                        }
                    }
                    Some(&"verifier") => verifier += 1,
                    _ => asserted += 1,
                }
            }

            let substantive = attested + verifier + asserted;
            let read_recommendation = if blocked {
                "blocked"
            } else if lane_contradictions > 0 {
                "read-adjudicate"
            } else if substantive > 0
                && asserted == 0
                && warnings == 0
                && !claimed_identity
                && !label_backed_attestation
            {
                "skip-verified"
            } else if substantive == 0 && warnings == 0 && !claimed_identity {
                // Infra-only lanes (e.g. the orchestrator's receipts).
                "skip-verified"
            } else {
                "read-unverified"
            };

            crate::schema::LaneDigest {
                lane,
                agent_id: records.iter().find_map(|record| record.agent_id.clone()),
                records: records.len() as u32,
                attested,
                verifier,
                asserted,
                warnings,
                contradictions: lane_contradictions,
                read_recommendation: read_recommendation.to_string(),
                drill_down,
            }
        })
        .collect()
}

/// Phase 2: the derived worklist. Every item tells Prime what to DO, carries
/// a category, a compiler-authored `blocking` flag, and (when clearable by
/// judgment) a resolution linkage. `suggested_argv` uses only engine tokens -
/// agent text never reaches it (finding 13); "<run-dir>" is a placeholder
/// Prime substitutes.
fn derive_worklist(
    evidence: &[EvidenceRecord],
    findings: &[VerifierFinding],
    contradictions: &[Contradiction],
    trusted_facts: &[CompiledFact],
    resolutions: &[ResolutionRecord],
) -> Vec<CandidateAction> {
    let mut resolved_by: HashMap<&str, &ResolutionRecord> = HashMap::new();
    for resolution in resolutions {
        resolved_by.insert(resolution.target_id.as_str(), resolution);
    }
    let fact_ids: BTreeSet<String> = trusted_facts.iter().map(|fact| fact.id.clone()).collect();

    let mut items: Vec<CandidateAction> = Vec::new();
    let mut push = |id: String,
                    category: &str,
                    blocking: bool,
                    target: &str,
                    title: String,
                    rationale: String,
                    source_ids: Vec<String>,
                    suggested_argv: Vec<String>| {
        let resolution = resolved_by.get(target);
        items.push(CandidateAction {
            id,
            title,
            rationale,
            actionability_score: if blocking { 0.9 } else { 0.5 },
            decision_dependency_ids: vec![target.to_string()],
            source_ids,
            category: Some(category.to_string()),
            blocking: Some(blocking),
            resolved: Some(resolution.is_some()),
            resolution_id: resolution.map(|r| r.id.clone()),
            suggested_argv,
        });
    };

    for contradiction in contradictions {
        let blocking =
            contradiction.id.starts_with("con:receipt:") || contradiction.severity == "high";
        push(
            format!("wl:adjudicate:{}", contradiction.id),
            "adjudicate",
            blocking,
            &contradiction.id,
            format!("Adjudicate contradiction {}", contradiction.id),
            contradiction.summary.clone(),
            contradiction.source_ids.clone(),
            vec![],
        );
    }
    for record in evidence.iter().filter(|record| record.kind == "blocker") {
        push(
            format!("wl:unblock:{}", record.id),
            "unblock",
            true,
            &record.id,
            format!(
                "Unblock lane {}",
                record.lane.as_deref().unwrap_or("(unknown)")
            ),
            record.summary.clone(),
            record.source_ids.clone(),
            vec![],
        );
    }
    for finding in findings.iter().filter(|finding| {
        finding.status != "passed" && finding.finding_kind.as_deref() != Some("synthesis")
    }) {
        push(
            format!("wl:resolve-finding:{}", finding.id),
            "resolve-finding",
            true,
            &finding.id,
            format!("Resolve non-passing finding {}", finding.id),
            finding.summary.clone(),
            finding.source_ids.clone(),
            vec![],
        );
    }
    const LOAD_BEARING: &[&str] = &["code-change", "test-change", "root-cause"];
    for record in evidence.iter().filter(|record| {
        LOAD_BEARING.contains(&record.kind.as_str())
            && !fact_ids.contains(&format!("fact:{}", record.id))
            && !resolved_by.contains_key(record.id.as_str())
    }) {
        let cited_label = record
            .source_ids
            .iter()
            .find(|id| id.starts_with("command:") || id.starts_with("test:"));
        let suggested = cited_label
            .map(|label| {
                vec![
                    "receipts".to_string(),
                    "run".to_string(),
                    "--run-dir".to_string(),
                    "<run-dir>".to_string(),
                    "--label".to_string(),
                    label.clone(),
                    "--".to_string(),
                ]
            })
            .unwrap_or_default();
        push(
            format!("wl:verify-claim:{}", record.id),
            "verify-claim",
            false,
            &record.id,
            format!("Verify load-bearing claim {}", record.id),
            record.summary.clone(),
            record.source_ids.clone(),
            suggested,
        );
    }
    for record in evidence
        .iter()
        .filter(|record| record.kind == "unstructured")
    {
        // Field tuning (2026-07-13 NTM run): a lane that ALSO delivered
        // structured or harvested records just has a prose remainder -
        // advising Prime to "re-task" it is noise. The advisory is for
        // lanes whose ENTIRE output resisted structuring.
        let lane_has_structured = record.lane.as_deref().is_some_and(|lane| {
            evidence.iter().any(|other| {
                other.lane.as_deref() == Some(lane)
                    && other.kind != "unstructured"
                    && other.kind != "objective"
                    && other.kind != "subagent-session"
            })
        });
        if lane_has_structured {
            continue;
        }
        push(
            format!("wl:re-task:{}", record.id),
            "re-task-or-accept",
            false,
            &record.id,
            format!(
                "Re-task or accept unstructured lane {}",
                record.lane.as_deref().unwrap_or("(unknown)")
            ),
            record.summary.clone(),
            record.source_ids.clone(),
            vec![],
        );
    }

    items.sort_by(|left, right| {
        right
            .blocking
            .cmp(&left.blocking)
            .then_with(|| left.id.cmp(&right.id))
    });
    items
}

#[allow(dead_code)]
fn hypotheses_from_failures(findings: &[VerifierFinding]) -> Vec<Hypothesis> {
    findings
        .iter()
        .filter(|finding| finding.status != "passed")
        .map(|finding| Hypothesis {
            id: format!("hypothesis:{}", finding.id),
            statement: format!("Resolve verifier finding: {}", finding.summary),
            reported_confidence: None,
            verifier_score: Some(finding.verifier_score),
            source_ids: finding.source_ids.clone(),
        })
        .collect()
}

fn open_questions_from_failures(findings: &[VerifierFinding]) -> Vec<String> {
    findings
        .iter()
        .filter(|finding| finding.status != "passed")
        .map(|finding| format!("What concrete change resolves `{}`?", finding.summary))
        .collect()
}

/// Ready-to-halt requires BOTH: every finding passing AND no unresolved
/// blocking worklist item (Phase 2 - the compiler is the single author of
/// blocking classification; halt and gate consume it).
fn halt_signals_from_state(
    findings: &[VerifierFinding],
    worklist: &[CandidateAction],
    created_at: &str,
) -> Vec<HaltSignal> {
    let failed: Vec<&VerifierFinding> = findings
        .iter()
        .filter(|finding| finding.status != "passed")
        .collect();
    let unresolved_blocking = worklist
        .iter()
        .filter(|item| item.blocking == Some(true) && item.resolved != Some(true))
        .count();

    // Halt signals must cite at least one registered source (packet
    // validation); fall back to the seed objective artifact when the
    // contributing sets are empty.
    let with_fallback = |ids: Vec<String>| -> Vec<String> {
        if ids.is_empty() {
            vec!["raw:objective.md".to_string()]
        } else {
            ids
        }
    };

    if failed.is_empty() && unresolved_blocking == 0 {
        return vec![HaltSignal {
            id: format!("halt:{created_at}:ready"),
            kind: "ready-to-halt".to_string(),
            contribution: 1.0,
            rationale: "All verifier findings passing; no unresolved blocking worklist items."
                .to_string(),
            source_ids: with_fallback(dedupe_source_ids(
                findings
                    .iter()
                    .flat_map(|finding| finding.source_ids.clone()),
            )),
        }];
    }

    let blocking_sources = worklist
        .iter()
        .filter(|item| item.blocking == Some(true) && item.resolved != Some(true))
        .flat_map(|item| item.source_ids.clone());
    vec![HaltSignal {
        id: format!("halt:{created_at}:continue"),
        kind: "continue".to_string(),
        contribution: 1.0,
        rationale: format!(
            "{} non-passing finding(s), {unresolved_blocking} unresolved blocking worklist item(s).",
            failed.len()
        ),
        source_ids: with_fallback(dedupe_source_ids(
            failed
                .iter()
                .flat_map(|finding| finding.source_ids.clone())
                .chain(blocking_sources),
        )),
    }]
}

fn state_delta_from_evidence(evidence: &[EvidenceRecord]) -> Vec<StateDelta> {
    evidence
        .iter()
        .map(|item| StateDelta {
            id: format!("delta:{}", item.id),
            kind: "evidence-observed".to_string(),
            target_id: item.id.clone(),
            summary: item.summary.clone(),
        })
        .collect()
}

/// Preserve the original lossy `dedupe_sources` for internal compatibility.
/// `compile_run_dir` uses `dedupe_sources_strict` instead so the compiler fails
/// fast on divergent hashes, but library consumers that construct packets
/// manually can still opt into the soft behavior.
#[allow(dead_code)]
fn dedupe_sources(sources: &mut Vec<SourceRef>) {
    sources.sort_by(|left, right| left.source_id.cmp(&right.source_id));
    sources.dedup_by(|left, right| left.source_id == right.source_id);
}

fn dedupe_source_ids(source_ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();
    for id in source_ids {
        if set.insert(id.clone()) {
            out.push(id);
        }
    }
    out
}

fn fnv1a_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::{collect_record_file_variants, compile_run_dir, dedupe_sources_strict, fnv1a_hash};
    use crate::schema::{NextPassPacket, RECEIPTS_HASH_ALG, RECEIPTS_SCHEMA_VERSION, SourceRef};
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn compiles_fixture_run_dir() {
        let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let run_dir = repo_root.join("tests/fixtures/run-basic");
        let state_dir = run_dir.join("state");
        let _ = fs::remove_dir_all(&state_dir);

        let report = compile_run_dir(&run_dir).expect("compile fixture");

        assert!(report.snapshot_path.exists());
        assert!(report.packet_path.exists());
        assert!(report.decision_log_path.exists());
        assert_eq!(report.evidence_count, 2);
        assert_eq!(report.verifier_finding_count, 2);

        let packet_json = fs::read_to_string(&report.packet_path).expect("read packet");
        assert!(packet_json.contains("\"evidence\""));
        assert!(packet_json.contains("\"halt_signals\""));

        let packet: NextPassPacket =
            serde_json::from_str(&packet_json).expect("packet is valid NextPassPacket");
        assert_eq!(packet.schema_version, RECEIPTS_SCHEMA_VERSION);
        assert!(!packet.sources.is_empty(), "packet must include sources");

        // Schema 2: raw verifier input is self-authored and unsigned. It is
        // useful attribution, but cannot promote a claim without a different
        // authenticated executor principal.
        assert!(packet.trusted_facts.is_empty());
        assert_eq!(
            packet
                .trust_assessments
                .iter()
                .find(|assessment| assessment.subject_id == "ev-2")
                .map(|assessment| assessment.claim_status.as_str()),
            Some("asserted")
        );
        assert!(
            run_dir.join("state/input_fingerprint.json").exists(),
            "compiler must write the input fingerprint (F12)"
        );

        for source in &packet.sources {
            assert_eq!(source.hash_alg, RECEIPTS_HASH_ALG);
            assert_eq!(
                source.hash.len(),
                16,
                "hash must be 16-char fnv1a-64 digest"
            );
            assert!(
                source.hash.chars().all(|ch| ch.is_ascii_hexdigit()),
                "hash must be hex",
            );
            if source.kind == "raw" {
                assert!(
                    !source.path.contains('\\'),
                    "raw path `{}` leaked backslashes",
                    source.path,
                );
                assert!(
                    !source.path.starts_with('/') && !source.path.contains(':'),
                    "raw path `{}` leaked absolute/drive-letter prefix",
                    source.path,
                );
                assert!(
                    source.path.starts_with("raw/"),
                    "raw path `{}` must start with `raw/`",
                    source.path,
                );
                assert_eq!(
                    source.observed_at, "2026-04-21T00:00:00Z",
                    "raw observed_at must anchor to manifest.created_at, not placeholder literals",
                );
            }
        }

        let _ = fs::remove_dir_all(&state_dir);
    }

    #[test]
    fn compile_detects_tampered_declared_file_ref() {
        let tmp = std::env::temp_dir().join("receipts-compile-tamper");
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("raw")).unwrap();
        fs::create_dir_all(tmp.join("worker-results")).unwrap();
        fs::create_dir_all(tmp.join("verifier-results")).unwrap();

        fs::write(
            tmp.join("manifest.json"),
            serde_json::json!({
                "objective_id": "obj-tamper",
                "run_id": "run-tamper",
                "branch_id": "main",
                "pass_id": "pass-0001",
                "created_at": "2026-04-21T00:00:00Z"
            })
            .to_string(),
        )
        .unwrap();
        fs::write(tmp.join("task.md"), "Tamper detection task").unwrap();

        let target_rel = "tamper-target.md";
        fs::write(tmp.join(target_rel), "real content\n").unwrap();
        // Deliberately write the wrong hash to simulate a forged declared ref.
        let evidence = serde_json::json!({
            "id": "ev-tampered",
            "kind": "root-cause",
            "summary": "hash must match file contents",
            "source_ids": [format!("file:{target_rel}"), "raw:objective.md"],
            "source_refs": [{
                "source_id": format!("file:{target_rel}"),
                "path": target_rel,
                "kind": "file",
                "hash": "deadbeefdeadbeef",
                "hash_alg": "fnv1a-64",
                "span": "1",
                "observed_at": "2026-04-21T00:00:00Z"
            }],
            "observed_at": "2026-04-21T00:00:00Z"
        });
        fs::write(
            tmp.join("raw/objective.md"),
            "# Objective\nTamper detection objective\n",
        )
        .unwrap();
        fs::write(
            tmp.join("worker-results/evidence.jsonl"),
            format!("{}\n", evidence),
        )
        .unwrap();
        fs::write(tmp.join("verifier-results/findings.jsonl"), "").unwrap();

        let err = compile_run_dir(&tmp).expect_err("tampered hash must fail compile");
        let msg = format!("{err}");
        assert!(
            msg.contains("hash mismatch"),
            "expected hash mismatch error, got: {msg}",
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn compile_versions_temporal_file_refs_when_repo_content_drifted() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tmp = std::env::temp_dir().join(format!("receipts-temporal-file-run-{nonce}"));
        let repo_root = std::env::temp_dir().join(format!("receipts-temporal-file-repo-{nonce}"));
        let source_path = "agent-tools/map.ts";
        let source_id = format!("file:{source_path}:1");
        let old_content = b"export const authority = 'old';\n";
        let current_content = b"export const authority = 'current';\n";
        let old_hash = fnv1a_hash(old_content);
        let current_hash = fnv1a_hash(current_content);

        fs::create_dir_all(tmp.join("raw")).unwrap();
        fs::create_dir_all(tmp.join("worker-results")).unwrap();
        fs::create_dir_all(tmp.join("verifier-results")).unwrap();
        fs::create_dir_all(repo_root.join("agent-tools")).unwrap();
        fs::write(repo_root.join(source_path), current_content).unwrap();
        fs::write(
            tmp.join("manifest.json"),
            serde_json::json!({
                "objective_id": "obj-temporal-file",
                "run_id": "run-temporal-file",
                "branch_id": "main",
                "pass_id": "pass-0001",
                "created_at": "2026-07-13T06:00:00Z",
                "repo_root": repo_root
            })
            .to_string(),
        )
        .unwrap();
        fs::write(tmp.join("task.md"), "Preserve temporal file evidence").unwrap();
        fs::write(
            tmp.join("raw/objective.md"),
            "# Objective\nPreserve temporal file evidence\n",
        )
        .unwrap();

        let evidence = [
            serde_json::json!({
                "id": "ev-before-edit",
                "kind": "observation",
                "summary": "observed the file before the edit",
                "source_ids": [source_id.clone()],
                "source_refs": [{
                    "source_id": source_id.clone(),
                    "path": source_path,
                    "kind": "file",
                    "hash": old_hash,
                    "hash_alg": "fnv1a-64",
                    "span": "1",
                    "observed_at": "2026-07-13T06:01:00Z"
                }],
                "observed_at": "2026-07-13T06:01:00Z"
            }),
            serde_json::json!({
                "id": "ev-after-edit",
                "kind": "observation",
                "summary": "observed the file after the edit",
                "source_ids": [source_id.clone()],
                "source_refs": [{
                    "source_id": source_id.clone(),
                    "path": source_path,
                    "kind": "file",
                    "hash": current_hash,
                    "hash_alg": "fnv1a-64",
                    "span": "1",
                    "observed_at": "2026-07-13T06:02:00Z"
                }],
                "observed_at": "2026-07-13T06:02:00Z"
            }),
        ];
        fs::write(
            tmp.join("worker-results/evidence.jsonl"),
            format!("{}\n{}\n", evidence[0], evidence[1]),
        )
        .unwrap();
        let finding = serde_json::json!({
            "id": "vf-after-edit",
            "summary": "verified the current file",
            "status": "failed",
            "verifier_score": 0.0,
            "source_ids": [source_id.clone()],
            "source_refs": [{
                "source_id": source_id.clone(),
                "path": source_path,
                "kind": "file",
                "hash": current_hash,
                "hash_alg": "fnv1a-64",
                "span": "1",
                "observed_at": "2026-07-13T06:03:00Z"
            }]
        });
        fs::write(
            tmp.join("verifier-results/findings.jsonl"),
            format!("{finding}\n"),
        )
        .unwrap();

        let report = compile_run_dir(&tmp).expect("temporal file refs must compile");
        let packet: NextPassPacket =
            serde_json::from_slice(&fs::read(report.packet_path).expect("read compiled packet"))
                .expect("packet schema");
        let expected_ids = [
            format!("{source_id}@fnv1a-64:{old_hash}"),
            format!("{source_id}@fnv1a-64:{current_hash}"),
        ];

        for expected_id in &expected_ids {
            assert!(
                packet
                    .sources
                    .iter()
                    .any(|source| source.source_id == *expected_id),
                "missing versioned source {expected_id}"
            );
        }
        for record in packet
            .evidence
            .iter()
            .filter(|record| record.id == "ev-before-edit" || record.id == "ev-after-edit")
        {
            assert_eq!(
                record.source_ids,
                vec![record.source_refs[0].source_id.clone()]
            );
            assert!(expected_ids.contains(&record.source_ids[0]));
        }
        let finding = packet
            .verifier_findings
            .iter()
            .find(|finding| finding.id == "vf-after-edit")
            .expect("compiled verifier finding");
        assert_eq!(
            finding.source_ids,
            vec![finding.source_refs[0].source_id.clone()]
        );
        assert_eq!(finding.source_ids[0], expected_ids[1]);

        let _ = fs::remove_dir_all(&tmp);
        let _ = fs::remove_dir_all(&repo_root);
    }

    #[test]
    fn temporal_file_versioning_rejects_same_record_hash_disagreement() {
        let source_id = "file:src/lib.rs:7";
        let refs = [
            SourceRef {
                source_id: source_id.to_string(),
                path: "src/lib.rs".to_string(),
                kind: "file".to_string(),
                hash: "1111111111111111".to_string(),
                hash_alg: RECEIPTS_HASH_ALG.to_string(),
                hash_basis: Some("content".to_string()),
                span: Some("7".to_string()),
                observed_at: "2026-07-13T06:01:00Z".to_string(),
            },
            SourceRef {
                source_id: source_id.to_string(),
                path: "src/lib.rs".to_string(),
                kind: "file".to_string(),
                hash: "2222222222222222".to_string(),
                hash_alg: RECEIPTS_HASH_ALG.to_string(),
                hash_basis: Some("content".to_string()),
                span: Some("7".to_string()),
                observed_at: "2026-07-13T06:01:00Z".to_string(),
            },
        ];
        let mut variants = BTreeMap::new();

        let err = collect_record_file_variants("evidence", "ev-bad", &refs, &mut variants)
            .expect_err("same record disagreement must fail");

        assert!(format!("{err}").contains("declares file source_id"));
    }

    #[test]
    fn strict_dedupe_still_rejects_divergent_non_file_sources() {
        let mut sources = vec![
            SourceRef {
                source_id: "command:test:focused".to_string(),
                path: "receipts/one.json".to_string(),
                kind: "command".to_string(),
                hash: "1111111111111111".to_string(),
                hash_alg: RECEIPTS_HASH_ALG.to_string(),
                hash_basis: Some("content".to_string()),
                span: None,
                observed_at: "2026-07-13T06:01:00Z".to_string(),
            },
            SourceRef {
                source_id: "command:test:focused".to_string(),
                path: "receipts/two.json".to_string(),
                kind: "command".to_string(),
                hash: "2222222222222222".to_string(),
                hash_alg: RECEIPTS_HASH_ALG.to_string(),
                hash_basis: Some("content".to_string()),
                span: None,
                observed_at: "2026-07-13T06:02:00Z".to_string(),
            },
        ];

        let err = dedupe_sources_strict(&mut sources)
            .expect_err("non-file source identities remain immutable");

        assert!(format!("{err}").contains("declared twice with divergent hash"));
    }
}
