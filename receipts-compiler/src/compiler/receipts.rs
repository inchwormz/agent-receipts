//! M1: the execution-receipt journal.
//!
//! `receipts run -- <cmd>` executes a command and appends a runtime-minted
//! `ReceiptRecord` to `<run-dir>/receipts/receipts.jsonl`. The journal is
//! hash-chained: each record's `record_hash` covers its own content (record
//! serialized without the `record_hash` field) and `prev_record_hash` links
//! to the previous record, so edits, deletions, and reorders are detectable
//! at compile time. Full stdout/stderr are stored content-addressed under
//! `receipts/artifacts/` with only bounded tails kept inline.
//!
//! Trust boundary: this module is invoked by the wrapper process, never by
//! agents; ingest downgrades agent-authored records that claim to be
//! receipts. Hash is fnv1a-64 (tamper tripwire, consistent with the rest of
//! the pipeline); BLAKE3 + signing are the M5 hardening step.

use crate::schema::ReceiptRecord;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const GENESIS: &str = "GENESIS";

/// The one label work receipts (`receipts diff`) carry. This label is
/// deliberately INVISIBLE to claim attestation: work receipts attest tree
/// state, never claims. Both the Rust compiler and the JS gate must exclude
/// it from passing-label logic and from citable receipt ids.
pub const WORK_LABEL: &str = "work:tree";

pub fn fnv1a_hash(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

/// Hash a receipt's content: the record serialized with `record_hash` blanked.
pub fn receipt_content_hash(record: &ReceiptRecord) -> String {
    let mut clone = record.clone();
    clone.record_hash = String::new();
    let canonical = serde_json::to_string(&clone).unwrap_or_default();
    fnv1a_hash(canonical.as_bytes())
}

pub fn journal_path(run_dir: &Path) -> PathBuf {
    run_dir.join("receipts").join("receipts.jsonl")
}

pub fn load_receipts(run_dir: &Path) -> Result<Vec<ReceiptRecord>, Box<dyn std::error::Error>> {
    let path = journal_path(run_dir);
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut receipts = Vec::new();
    for (index, line) in fs::read_to_string(&path)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: ReceiptRecord = serde_json::from_str(line)
            .map_err(|err| format!("receipts.jsonl line {} is not a receipt: {err}", index + 1))?;
        receipts.push(record);
    }
    Ok(receipts)
}

/// Verify the full chain: every record's content hash matches, and every
/// prev link points at the preceding record (GENESIS for the first). Returns
/// the verified records; a broken chain is a hard error - the journal is the
/// one artifact whose integrity is non-negotiable.
pub fn load_verified_receipts(
    run_dir: &Path,
) -> Result<Vec<ReceiptRecord>, Box<dyn std::error::Error>> {
    let receipts = load_receipts(run_dir)?;
    let mut prev = GENESIS.to_string();
    for (index, record) in receipts.iter().enumerate() {
        let actual = receipt_content_hash(record);
        if record.record_hash != actual {
            return Err(format!(
                "receipt chain broken at entry {} ({}): record_hash {} does not match content hash {} - the journal was edited after minting",
                index + 1,
                record.id,
                record.record_hash,
                actual
            )
            .into());
        }
        if record.prev_record_hash != prev {
            return Err(format!(
                "receipt chain broken at entry {} ({}): prev_record_hash {} does not link to preceding record hash {}",
                index + 1,
                record.id,
                record.prev_record_hash,
                prev
            )
            .into());
        }
        prev = record.record_hash.clone();
    }
    Ok(receipts)
}

/// Append a receipt under an advisory lock (O_EXCL sidecar, same discipline
/// as JS ingest). Fills id, prev_record_hash, and record_hash; returns the
/// completed record.
pub fn append_receipt(
    run_dir: &Path,
    mut record: ReceiptRecord,
) -> Result<ReceiptRecord, Box<dyn std::error::Error>> {
    let path = journal_path(run_dir);
    fs::create_dir_all(path.parent().expect("journal parent"))?;
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

    let result = (|| -> Result<ReceiptRecord, Box<dyn std::error::Error>> {
        let existing = load_receipts(run_dir)?;
        record.id = format!("rcpt-{:04}", existing.len() + 1);
        record.prev_record_hash = existing
            .last()
            .map(|last| last.record_hash.clone())
            .unwrap_or_else(|| GENESIS.to_string());
        record.record_hash = String::new();
        record.record_hash = receipt_content_hash(&record);

        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        serde_json::to_writer(&mut file, &record)?;
        file.write_all(b"\n")?;
        Ok(record)
    })();

    let _ = fs::remove_file(&lock_path);
    result
}

/// Store full command output content-addressed; returns (hash, relative path).
pub fn store_artifact(
    run_dir: &Path,
    bytes: &[u8],
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let hash = fnv1a_hash(bytes);
    let dir = run_dir.join("receipts").join("artifacts");
    fs::create_dir_all(&dir)?;
    let rel = format!("receipts/artifacts/{hash}.txt");
    let path = run_dir.join(&rel);
    if path.exists() {
        // Content-address squatting defense: fnv1a is a weak hash, and a lane
        // could pre-plant a file under a hash a future artifact will get.
        // Byte-compare on collision; mismatch is a hard integrity error.
        let existing = fs::read(&path)?;
        if existing != bytes {
            return Err(format!(
                "artifact collision at {rel}: existing bytes differ from new content under the same hash - refusing to proceed (possible content-address squatting)"
            )
            .into());
        }
    } else {
        fs::write(&path, bytes)?;
    }
    Ok((hash, rel))
}

/// Best-effort git tree fingerprint of repo_root: "HEAD:<sha> dirty:<hash of
/// porcelain status>". None when repo_root is absent or not a git repo.
pub fn git_tree_state(repo_root: Option<&str>) -> Option<String> {
    let root = repo_root?;
    let head = std::process::Command::new("git")
        .args(["-C", root, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !head.status.success() {
        return None;
    }
    let head_sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let status = std::process::Command::new("git")
        .args(["-C", root, "status", "--porcelain"])
        .output()
        .ok()?;
    let dirty_hash = fnv1a_hash(&status.stdout);
    Some(format!("{head_sha} dirty:{dirty_hash}"))
}

#[cfg(test)]
mod tests {
    use super::{append_receipt, load_verified_receipts, receipt_content_hash};
    use crate::schema::ReceiptRecord;
    use std::fs;

    fn blank_receipt(exit_code: i64) -> ReceiptRecord {
        ReceiptRecord {
            id: String::new(),
            label: Some("test:demo".to_string()),
            cmd: vec!["echo".to_string(), "hi".to_string()],
            cwd: ".".to_string(),
            exit_code,
            duration_ms: 5,
            started_at: "2026-07-12T00:00:00Z".to_string(),
            ended_at: "2026-07-12T00:00:01Z".to_string(),
            stdout_hash: "0000000000000000".to_string(),
            stderr_hash: "0000000000000000".to_string(),
            stdout_tail: "hi".to_string(),
            stderr_tail: String::new(),
            tree_before: None,
            tree_after: None,
            lane: Some("orchestrator".to_string()),
            agent_id: Some("prime".to_string()),
            writer: "receipts-test".to_string(),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        }
    }

    fn temp_run_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("receipts-rcpt-{tag}-{nanos}"));
        fs::create_dir_all(&dir).expect("temp run dir");
        dir
    }

    #[test]
    fn chain_links_and_verifies() {
        let dir = temp_run_dir("chain");
        let first = append_receipt(&dir, blank_receipt(0)).expect("append first");
        let second = append_receipt(&dir, blank_receipt(1)).expect("append second");
        assert_eq!(first.id, "rcpt-0001");
        assert_eq!(second.prev_record_hash, first.record_hash);
        let verified = load_verified_receipts(&dir).expect("chain verifies");
        assert_eq!(verified.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_journal_breaks_the_chain() {
        let dir = temp_run_dir("tamper");
        append_receipt(&dir, blank_receipt(0)).expect("append");
        let path = dir.join("receipts").join("receipts.jsonl");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("\"exit_code\":0", "\"exit_code\":1");
        fs::write(&path, text).unwrap();
        let err = load_verified_receipts(&dir).expect_err("tamper must break chain");
        assert!(format!("{err}").contains("chain broken"), "got: {err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn receipt_record_is_frozen_canary() {
        // The hash chain preimage is the serialized record. ADDING ANY FIELD
        // to ReceiptRecord makes version-skewed verifiers (which drop unknown
        // fields on deserialize) compute different content hashes for valid
        // journals and report false tampering. The record is FROZEN:
        // extensions live in content-addressed artifacts. If this test fails,
        // you added a field - move it to an artifact instead.
        let mut record = blank_receipt(0);
        record.id = "rcpt-0001".to_string();
        record.prev_record_hash = "GENESIS".to_string();
        record.record_hash = String::new();
        let serialized = serde_json::to_string(&record).expect("serialize");
        let expected = "{\"id\":\"rcpt-0001\",\"label\":\"test:demo\",\"cmd\":[\"echo\",\"hi\"],\"cwd\":\".\",\"exit_code\":0,\"duration_ms\":5,\"started_at\":\"2026-07-12T00:00:00Z\",\"ended_at\":\"2026-07-12T00:00:01Z\",\"stdout_hash\":\"0000000000000000\",\"stderr_hash\":\"0000000000000000\",\"stdout_tail\":\"hi\",\"stderr_tail\":\"\",\"lane\":\"orchestrator\",\"agent_id\":\"prime\",\"writer\":\"receipts-test\",\"prev_record_hash\":\"GENESIS\",\"record_hash\":\"\"}";
        assert_eq!(
            serialized, expected,
            "ReceiptRecord serialization changed - the record is FROZEN (see receipt_record_is_frozen_canary comment)"
        );
    }

    #[test]
    fn artifact_collision_with_different_bytes_is_fatal() {
        let dir = temp_run_dir("squat");
        let (_hash, rel) = super::store_artifact(&dir, b"honest content").expect("store");
        // Overwrite the stored artifact to simulate a pre-planted file, then
        // store the same honest content again - it lands on the same path and
        // must detect the byte mismatch.
        fs::write(dir.join(&rel), b"planted content").unwrap();
        let err = super::store_artifact(&dir, b"honest content")
            .expect_err("collision with different bytes must fail");
        assert!(format!("{err}").contains("squatting"), "got: {err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn content_hash_ignores_record_hash_field() {
        let mut record = blank_receipt(0);
        record.record_hash = "aaaaaaaaaaaaaaaa".to_string();
        let first = receipt_content_hash(&record);
        record.record_hash = "bbbbbbbbbbbbbbbb".to_string();
        assert_eq!(first, receipt_content_hash(&record));
    }
}
