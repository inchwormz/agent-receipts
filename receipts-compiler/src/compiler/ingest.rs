//! Forgiving subagent ingest with an engine-owned trust boundary.
//!
//! Input format is a convenience, never authority. The engine quarantines
//! the exact lane output, overrides caller attribution, discards caller-made
//! source refs, reconstructs verifiable refs, and falls back to bounded prose
//! harvesting when no machine records are present.

use crate::compiler::receipts::{fnv1a_hash, load_verified_receipts};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct IngestReport {
    pub ok: bool,
    pub lane: String,
    pub agent_id: String,
    pub raw_source_id: String,
    pub evidence_records: usize,
    pub verifier_records: usize,
    pub harvested: usize,
    pub unstructured: bool,
    pub blocked: bool,
    pub repairs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordRoute {
    Evidence,
    Verifier,
}

#[derive(Debug)]
struct ParsedRecord {
    value: Value,
    route: RecordRoute,
}

struct IngestLock {
    path: PathBuf,
}

impl Drop for IngestLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_lock(run_dir: &Path) -> Result<IngestLock, Box<dyn std::error::Error>> {
    let path = run_dir.join("ingest.lock");
    for _ in 0..200 {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(IngestLock { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err("timed out acquiring ingest.lock".into())
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    slug.trim_matches('-').to_string()
}

fn normalized_relative(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn route_for(value: &Value, hinted: RecordRoute) -> RecordRoute {
    if hinted == RecordRoute::Verifier
        || value.get("status").is_some()
        || value.get("verifier_score").is_some()
    {
        RecordRoute::Verifier
    } else {
        RecordRoute::Evidence
    }
}

fn parse_json_values(text: &str) -> Vec<Value> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return match value {
            Value::Array(values) => values,
            value => vec![value],
        };
    }
    trimmed
        .lines()
        .filter_map(|line| {
            let repaired = repair_json_like(line.trim());
            serde_json::from_str(&repaired).ok()
        })
        .collect()
}

fn repair_json_like(value: &str) -> String {
    let mut repaired = value.trim().to_string();
    while repaired.contains(",}") || repaired.contains(",]") {
        repaired = repaired.replace(",}", "}").replace(",]", "]");
    }
    if repaired.contains('\'') && !repaired.contains('"') {
        repaired = repaired.replace('\'', "\"");
    }
    quote_bare_object_keys(&repaired)
}

fn quote_bare_object_keys(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 8);
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            index += 1;
            continue;
        }
        if ch == '{' || ch == ',' {
            out.push(ch);
            index += 1;
            while index < chars.len() && chars[index].is_whitespace() {
                out.push(chars[index]);
                index += 1;
            }
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '_' | '-'))
            {
                index += 1;
            }
            let mut probe = index;
            while probe < chars.len() && chars[probe].is_whitespace() {
                probe += 1;
            }
            if index > start && probe < chars.len() && chars[probe] == ':' {
                out.push('"');
                out.extend(chars[start..index].iter());
                out.push('"');
            } else {
                out.extend(chars[start..index].iter());
            }
            continue;
        }
        out.push(ch);
        index += 1;
    }
    out
}

fn parse_records(text: &str) -> Vec<ParsedRecord> {
    let lines: Vec<&str> = text.lines().collect();
    let mut records = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        if !line.starts_with("```") {
            index += 1;
            continue;
        }
        let language = line.trim_start_matches('`').trim().to_ascii_lowercase();
        let hint = if language.contains("verifier") {
            RecordRoute::Verifier
        } else {
            RecordRoute::Evidence
        };
        index += 1;
        let mut block = String::new();
        while index < lines.len() && !lines[index].trim().starts_with("```") {
            block.push_str(lines[index]);
            block.push('\n');
            index += 1;
        }
        index += usize::from(index < lines.len());
        for value in parse_json_values(&block) {
            if value.is_object() {
                records.push(ParsedRecord {
                    route: route_for(&value, hint),
                    value,
                });
            }
        }
    }
    records
}

fn prose_claims(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.len() >= 12 && line.len() <= 500)
        .filter(|line| line.chars().any(char::is_alphabetic))
        .filter(|line| !line.starts_with('#') && !line.starts_with("```"))
        .filter(|line| !line.to_ascii_uppercase().starts_with("BLOCKED"))
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.starts_with("lane:")
                && !lower.starts_with("agent_id:")
                && !lower.contains("passed/total")
        })
        .take(24)
        .map(str::to_string)
        .collect()
}

fn blocked_reason(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let reason = trimmed
            .strip_prefix("BLOCKED")?
            .trim_start_matches([':', '-', ' '])
            .trim();
        (reason.len() >= 4 && reason.chars().any(char::is_alphabetic)).then(|| reason.to_string())
    })
}

fn append_jsonl(path: &Path, values: &[Value]) -> Result<(), Box<dyn std::error::Error>> {
    if values.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(path.parent().ok_or("JSONL path has no parent")?)?;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    for value in values {
        serde_json::to_writer(&mut file, value)?;
        file.write_all(b"\n")?;
    }
    file.flush()?;
    Ok(())
}

fn raw_source_ref(source_id: &str, relative_path: &str, bytes: &[u8], observed_at: &str) -> Value {
    serde_json::json!({
        "source_id": source_id,
        "path": relative_path,
        "kind": "raw",
        "hash": fnv1a_hash(bytes),
        "hash_alg": "fnv1a-64",
        "hash_basis": "content",
        "observed_at": observed_at,
    })
}

fn parse_file_source(source_id: &str) -> (&str, Option<String>) {
    let rest = source_id.strip_prefix("file:").unwrap_or(source_id);
    if let Some((path, span)) = rest.rsplit_once(':') {
        if span
            .split('-')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return (path, Some(span.to_string()));
        }
    }
    (rest, None)
}

fn source_ref_for(
    source_id: &str,
    run_dir: &Path,
    repo_root: &Path,
    observed_at: &str,
    receipt_ids: &BTreeSet<String>,
) -> Result<(String, Value, Option<String>), Box<dyn std::error::Error>> {
    if source_id.starts_with("file:") {
        let (declared, span) = parse_file_source(source_id);
        let candidate = if Path::new(declared).is_absolute() {
            PathBuf::from(declared)
        } else {
            repo_root.join(declared)
        };
        let canonical = fs::canonicalize(&candidate);
        if let Ok(canonical) = canonical {
            let canonical_repo = fs::canonicalize(repo_root)?;
            let canonical_run = fs::canonicalize(run_dir)?;
            if canonical.starts_with(&canonical_repo) && !canonical.starts_with(&canonical_run) {
                let relative = canonical.strip_prefix(&canonical_repo)?;
                let relative = normalized_relative(relative);
                let normalized_id = span
                    .as_ref()
                    .map(|span| format!("file:{relative}:{span}"))
                    .unwrap_or_else(|| format!("file:{relative}"));
                let bytes = fs::read(&canonical)?;
                return Ok((
                    normalized_id.clone(),
                    serde_json::json!({
                        "source_id": normalized_id,
                        "path": relative,
                        "kind": "file",
                        "hash": fnv1a_hash(&bytes),
                        "hash_alg": "fnv1a-64",
                        "hash_basis": "content",
                        "span": span,
                        "observed_at": observed_at,
                    }),
                    None,
                ));
            }
        }
        let demoted = format!("log:unverifiable-{}", fnv1a_hash(source_id.as_bytes()));
        return Ok((
            demoted.clone(),
            label_source_ref(&demoted, observed_at),
            Some(format!("unverifiable-file-citation: {source_id}")),
        ));
    }
    if let Some(id) = source_id.strip_prefix("receipt:") {
        if !receipt_ids.contains(id) {
            let demoted = format!("log:unminted-receipt-{}", fnv1a_hash(source_id.as_bytes()));
            return Ok((
                demoted.clone(),
                label_source_ref(&demoted, observed_at),
                Some(format!("unminted-receipt-citation: {source_id}")),
            ));
        }
    }
    if let Some(commit) = source_id.strip_prefix("commit:") {
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
                .status()
                .is_ok_and(|status| status.success());
        if !valid {
            let demoted = format!(
                "log:unverifiable-commit-{}",
                fnv1a_hash(source_id.as_bytes())
            );
            return Ok((
                demoted.clone(),
                label_source_ref(&demoted, observed_at),
                Some(format!("unverifiable-commit-citation: {source_id}")),
            ));
        }
        return Ok((
            source_id.to_string(),
            serde_json::json!({
                "source_id": source_id,
                "path": source_id,
                "kind": "commit",
                "hash": fnv1a_hash(source_id.as_bytes()),
                "hash_alg": "fnv1a-64",
                "hash_basis": "git",
                "observed_at": observed_at,
            }),
            None,
        ));
    }
    Ok((
        source_id.to_string(),
        label_source_ref(source_id, observed_at),
        None,
    ))
}

fn label_source_ref(source_id: &str, observed_at: &str) -> Value {
    let kind = source_id.split(':').next().unwrap_or("log");
    serde_json::json!({
        "source_id": source_id,
        "path": source_id,
        "kind": kind,
        "hash": fnv1a_hash(source_id.as_bytes()),
        "hash_alg": "fnv1a-64",
        "hash_basis": "label",
        "observed_at": observed_at,
    })
}

fn normalize_record(
    mut value: Value,
    route: RecordRoute,
    index: usize,
    lane: &str,
    agent_id: &str,
    raw_source_id: &str,
    raw_ref: &Value,
    observed_at: &str,
    run_dir: &Path,
    repo_root: &Path,
    receipt_ids: &BTreeSet<String>,
) -> Result<Value, Box<dyn std::error::Error>> {
    let object = value
        .as_object_mut()
        .ok_or("ingested record must be a JSON object")?;
    if !object.contains_key("summary") {
        if let Some(text) = object.remove("text") {
            object.insert("summary".to_string(), text);
        }
    }
    if route == RecordRoute::Evidence && !object.contains_key("kind") {
        if let Some(kind) = object.remove("type") {
            object.insert("kind".to_string(), kind);
        } else {
            object.insert("kind".to_string(), Value::String("observation".to_string()));
        }
    }
    if !object.contains_key("id") {
        object.insert(
            "id".to_string(),
            Value::String(format!(
                "{}-{}-{index:04}",
                if route == RecordRoute::Verifier {
                    "vf"
                } else {
                    "ev"
                },
                slugify(lane)
            )),
        );
    }
    if let Some(confidence) = object.remove("confidence") {
        object.insert("reported_confidence".to_string(), confidence);
    }
    if let Some(claimed) = object.get("agent_id").and_then(Value::as_str) {
        if claimed != agent_id {
            object.insert(
                "claimed_agent_id".to_string(),
                Value::String(claimed.to_string()),
            );
        }
    }
    if let Some(claimed) = object.get("lane").and_then(Value::as_str) {
        if claimed != lane {
            object.insert(
                "claimed_lane".to_string(),
                Value::String(claimed.to_string()),
            );
        }
    }
    object.insert("agent_id".to_string(), Value::String(agent_id.to_string()));
    object.insert("lane".to_string(), Value::String(lane.to_string()));
    object
        .entry("observed_at".to_string())
        .or_insert_with(|| Value::String(observed_at.to_string()));
    object.remove("source_refs");

    let source_value = object
        .remove("source_ids")
        .or_else(|| object.remove("sources"));
    let mut source_ids: Vec<String> = match source_value {
        Some(Value::Array(values)) => values
            .into_iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(value)) => vec![value],
        _ => Vec::new(),
    };
    if !source_ids.iter().any(|source| source == raw_source_id) {
        source_ids.insert(0, raw_source_id.to_string());
    }
    let mut refs = vec![raw_ref.clone()];
    let mut normalized_ids = vec![raw_source_id.to_string()];
    let mut warnings = Vec::new();
    for source_id in source_ids {
        if source_id == raw_source_id {
            continue;
        }
        let (normalized, source_ref, warning) =
            source_ref_for(&source_id, run_dir, repo_root, observed_at, receipt_ids)?;
        if !normalized_ids.contains(&normalized) {
            normalized_ids.push(normalized);
            refs.push(source_ref);
        }
        if let Some(warning) = warning {
            warnings.push(Value::String(warning));
        }
    }
    object.insert("source_ids".to_string(), serde_json::json!(normalized_ids));
    object.insert("source_refs".to_string(), Value::Array(refs));
    if !warnings.is_empty() {
        object.insert("provenance_warnings".to_string(), Value::Array(warnings));
    }
    if route == RecordRoute::Verifier {
        object
            .entry("status".to_string())
            .or_insert_with(|| Value::String("pending".to_string()));
        object
            .entry("verifier_score".to_string())
            .or_insert_with(|| serde_json::json!(0.0));
    }
    Ok(value)
}

pub fn ingest_subagent(
    run_dir: &Path,
    lane: &str,
    agent_id: &str,
    from: &Path,
    observed_at: &str,
    stamp: &str,
) -> Result<IngestReport, Box<dyn std::error::Error>> {
    if lane.trim().is_empty() || agent_id.trim().is_empty() {
        return Err("ingest requires non-empty lane and agent_id".into());
    }
    let input = fs::read_to_string(from)?;
    if input.trim().is_empty() {
        return Err("subagent output is empty".into());
    }
    let _lock = acquire_lock(run_dir)?;
    let manifest: Value = serde_json::from_slice(&fs::read(run_dir.join("manifest.json"))?)?;
    let repo_root = PathBuf::from(
        manifest
            .get("repo_root")
            .and_then(Value::as_str)
            .ok_or("manifest.json has no repo_root")?,
    );
    let raw_dir = run_dir.join("raw/subagents");
    fs::create_dir_all(&raw_dir)?;
    let direct_raw = fs::canonicalize(from)
        .ok()
        .zip(fs::canonicalize(&raw_dir).ok())
        .is_some_and(|(from, raw)| from.starts_with(raw));
    let raw_path = if direct_raw {
        from.to_path_buf()
    } else {
        raw_dir.join(format!(
            "{}-{}-{}-{}.md",
            stamp,
            std::process::id(),
            slugify(lane),
            slugify(agent_id)
        ))
    };
    if !direct_raw {
        fs::write(
            &raw_path,
            format!(
                "# Subagent Session {stamp}\n\nlane: {lane}\nagent_id: {agent_id}\n\n{}\n",
                input.trim()
            ),
        )?;
    }
    let raw_bytes = fs::read(&raw_path)?;
    let raw_text = String::from_utf8(raw_bytes.clone())?;
    let raw_name = raw_path
        .strip_prefix(&raw_dir)?
        .to_string_lossy()
        .replace('\\', "/");
    let raw_source_id = format!("raw:subagents/{raw_name}");
    let relative_raw_path = normalized_relative(raw_path.strip_prefix(run_dir)?);

    let existing_evidence =
        fs::read_to_string(run_dir.join("worker-results/evidence.jsonl")).unwrap_or_default();
    for line in existing_evidence
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        if let Ok(record) = serde_json::from_str::<Value>(line) {
            let duplicate = record.get("kind").and_then(Value::as_str) == Some("subagent-session")
                && record
                    .get("source_ids")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(&raw_source_id))
                    });
            if duplicate {
                return Err(format!(
                    "duplicate ingest: {raw_source_id} already has a subagent-session record"
                )
                .into());
            }
        }
    }

    let raw_ref = raw_source_ref(&raw_source_id, &relative_raw_path, &raw_bytes, observed_at);
    let receipt_ids: BTreeSet<String> = load_verified_receipts(run_dir)?
        .into_iter()
        .map(|receipt| receipt.id)
        .collect();
    let mut parsed = parse_records(&raw_text);
    let mut harvested = 0;
    let mut unstructured = false;
    if parsed.is_empty() {
        let claims = prose_claims(&input);
        harvested = claims.len();
        for (index, claim) in claims.into_iter().enumerate() {
            parsed.push(ParsedRecord {
                route: RecordRoute::Evidence,
                value: serde_json::json!({
                    "id": format!("ev-harvested-{}-{index:04}", slugify(lane)),
                    "kind": "observation",
                    "summary": claim,
                    "rationale": "harvested-from-prose",
                }),
            });
        }
        if parsed.is_empty() {
            unstructured = true;
            parsed.push(ParsedRecord {
                route: RecordRoute::Evidence,
                value: serde_json::json!({
                    "id": format!("ev-unstructured-{}", slugify(lane)),
                    "kind": "unstructured",
                    "summary": "Lane returned prose without machine-readable claim records",
                    "provenance_warnings": ["unstructured: content quarantined only"],
                }),
            });
        }
    }
    let blocked = blocked_reason(&input);
    if let Some(reason) = &blocked {
        parsed.push(ParsedRecord {
            route: RecordRoute::Evidence,
            value: serde_json::json!({
                "id": format!("ev-blocker-{}", slugify(lane)),
                "kind": "blocker",
                "summary": reason,
            }),
        });
    }

    let mut evidence = Vec::new();
    let mut findings = Vec::new();
    for (index, record) in parsed.into_iter().enumerate() {
        let normalized = normalize_record(
            record.value,
            record.route,
            index,
            lane,
            agent_id,
            &raw_source_id,
            &raw_ref,
            observed_at,
            run_dir,
            &repo_root,
            &receipt_ids,
        )?;
        match record.route {
            RecordRoute::Evidence => evidence.push(normalized),
            RecordRoute::Verifier => findings.push(normalized),
        }
    }
    evidence.push(serde_json::json!({
        "id": format!("ev-subagent-session-{}-{}", stamp, slugify(lane)),
        "kind": "subagent-session",
        "summary": format!("Subagent session quarantined for lane {lane}"),
        "source_ids": [raw_source_id.clone()],
        "source_refs": [raw_ref],
        "observed_at": observed_at,
        "agent_id": agent_id,
        "lane": lane,
    }));

    append_jsonl(&run_dir.join("worker-results/evidence.jsonl"), &evidence)?;
    append_jsonl(&run_dir.join("verifier-results/findings.jsonl"), &findings)?;
    Ok(IngestReport {
        ok: true,
        lane: lane.to_string(),
        agent_id: agent_id.to_string(),
        raw_source_id,
        evidence_records: evidence.len(),
        verifier_records: findings.len(),
        harvested,
        unstructured,
        blocked: blocked.is_some(),
        repairs: Vec::new(),
    })
}
