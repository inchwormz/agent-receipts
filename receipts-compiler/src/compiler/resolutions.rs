//! Phase 2: the resolutions journal - Prime's typed adjudication channel.
//!
//! Blocking worklist items (contradictions to adjudicate, blocked lanes,
//! failed findings) would otherwise be one-way ratchets over append-only
//! inputs: recomputed every compile, impossible to clear without hand-editing
//! quarantined evidence (adversarial-review finding 3). `receipts resolve`
//! appends a hash-chained resolution record to `decisions/resolutions.jsonl`;
//! compile consumes it and marks the matching worklist item resolved. The
//! journal is covered by the input fingerprint, so resolutions are custody-
//! tracked like every other input.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::compiler::receipts::{GENESIS, fnv1a_hash};

/// FROZEN like ReceiptRecord: the chain preimage is the serialized record.
/// Extensions belong in cited artifacts, not new fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolutionRecord {
    pub id: String,
    /// The worklist target being resolved: a contradiction id, a blocker
    /// evidence id, or a finding id.
    pub target_id: String,
    pub reason: String,
    /// Optional supporting citation (e.g. receipt:rcpt-0007).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cite: Option<String>,
    pub resolved_at: String,
    pub writer: String,
    pub prev_record_hash: String,
    pub record_hash: String,
}

pub fn resolution_content_hash(record: &ResolutionRecord) -> String {
    let mut clone = record.clone();
    clone.record_hash = String::new();
    fnv1a_hash(serde_json::to_string(&clone).unwrap_or_default().as_bytes())
}

pub fn resolutions_path(run_dir: &Path) -> PathBuf {
    run_dir.join("decisions").join("resolutions.jsonl")
}

pub fn load_resolutions(
    run_dir: &Path,
) -> Result<Vec<ResolutionRecord>, Box<dyn std::error::Error>> {
    let path = resolutions_path(run_dir);
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut records = Vec::new();
    for (index, line) in fs::read_to_string(&path)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: ResolutionRecord = serde_json::from_str(line).map_err(|err| {
            format!(
                "resolutions.jsonl line {} is not a resolution: {err}",
                index + 1
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

pub fn load_verified_resolutions(
    run_dir: &Path,
) -> Result<Vec<ResolutionRecord>, Box<dyn std::error::Error>> {
    let records = load_resolutions(run_dir)?;
    let mut prev = GENESIS.to_string();
    for (index, record) in records.iter().enumerate() {
        let actual = resolution_content_hash(record);
        if record.record_hash != actual {
            return Err(format!(
                "resolution chain broken at entry {} ({}): record_hash mismatch - the journal was edited after minting",
                index + 1,
                record.id
            )
            .into());
        }
        if record.prev_record_hash != prev {
            return Err(format!(
                "resolution chain broken at entry {} ({}): prev link mismatch",
                index + 1,
                record.id
            )
            .into());
        }
        prev = record.record_hash.clone();
    }
    Ok(records)
}

pub fn append_resolution(
    run_dir: &Path,
    mut record: ResolutionRecord,
) -> Result<ResolutionRecord, Box<dyn std::error::Error>> {
    let path = resolutions_path(run_dir);
    fs::create_dir_all(path.parent().expect("resolutions parent"))?;
    let lock_path = path.with_extension("jsonl.lock");

    let mut acquired = false;
    for _ in 0..200 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(_) => {
                acquired = true;
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
        }
    }
    if !acquired {
        return Err(format!("timed out acquiring {}", lock_path.display()).into());
    }

    let result = (|| -> Result<ResolutionRecord, Box<dyn std::error::Error>> {
        let existing = load_resolutions(run_dir)?;
        record.id = format!("res-{:04}", existing.len() + 1);
        record.prev_record_hash = existing
            .last()
            .map(|last| last.record_hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        record.record_hash = String::new();
        record.record_hash = resolution_content_hash(&record);

        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
        Ok(record)
    })();

    let _ = fs::remove_file(&lock_path);
    result
}

#[cfg(test)]
mod tests {
    use super::{ResolutionRecord, append_resolution, load_verified_resolutions};
    use std::fs;

    fn blank(target: &str) -> ResolutionRecord {
        ResolutionRecord {
            id: String::new(),
            target_id: target.to_string(),
            reason: "adjudicated by prime".to_string(),
            cite: None,
            resolved_at: "2026-07-13T00:00:00Z".to_string(),
            writer: "receipts-test".to_string(),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        }
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("receipts-res-{tag}-{nanos}"));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn chains_and_verifies() {
        let dir = temp_dir("chain");
        let first = append_resolution(&dir, blank("con:auto:a:b")).expect("first");
        let second = append_resolution(&dir, blank("ev-blocker-1")).expect("second");
        assert_eq!(first.id, "res-0001");
        assert_eq!(second.prev_record_hash, first.record_hash);
        assert_eq!(load_verified_resolutions(&dir).expect("verify").len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampering_breaks_the_chain() {
        let dir = temp_dir("tamper");
        append_resolution(&dir, blank("con:auto:a:b")).expect("append");
        let path = dir.join("decisions").join("resolutions.jsonl");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("adjudicated by prime", "totally legit");
        fs::write(&path, text).unwrap();
        let err = load_verified_resolutions(&dir).expect_err("must break");
        assert!(format!("{err}").contains("chain broken"), "got: {err}");
        let _ = fs::remove_dir_all(&dir);
    }
}
