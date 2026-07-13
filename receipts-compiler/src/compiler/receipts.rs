//! M1: the execution-receipt journal.
//!
//! `receipts run -- <cmd>` executes a command and appends a runtime-minted
//! a record to `<run-dir>/receipts/receipts.jsonl`. Frozen V1 lines remain
//! readable as `legacy_weak`; new lines are strict V2 envelopes whose BLAKE3
//! hash and Ed25519 signature cover run, sequence, previous digest, payload,
//! executor principal, and engine identity. There is one journal authority,
//! not a weak journal plus a stronger sidecar.
//!
//! Trust boundary: this module is invoked by the engine, never by agents;
//! ingest downgrades agent-authored records that claim to be receipts.

use crate::compiler::crypto::{
    DigestRef, SignedEngineIdentity, SignedReceiptEnvelope, sign_receipt_envelope,
    verify_receipt_envelope, verify_receipt_head, write_receipt_head,
};
use crate::schema::ReceiptRecord;
use serde_json::Value;
use std::collections::BTreeMap;
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

/// Hash a frozen V1 receipt's content. New records never use this function.
pub fn receipt_content_hash(record: &ReceiptRecord) -> String {
    let mut clone = record.clone();
    clone.record_hash = String::new();
    let canonical = serde_json::to_string(&clone).unwrap_or_default();
    fnv1a_hash(canonical.as_bytes())
}

pub fn journal_path(run_dir: &Path) -> PathBuf {
    run_dir.join("receipts").join("receipts.jsonl")
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceiptVerification {
    pub integrity: &'static str,
    pub hash_alg: &'static str,
    pub principal_id: Option<String>,
    pub engine: Option<SignedEngineIdentity>,
}

#[derive(Debug, Clone)]
pub struct VerifiedReceiptJournal {
    pub records: Vec<ReceiptRecord>,
    pub verification: BTreeMap<String, ReceiptVerification>,
}

enum ReceiptJournalLine {
    Legacy(ReceiptRecord),
    Signed(SignedReceiptEnvelope),
}

fn load_journal_lines(
    run_dir: &Path,
) -> Result<Vec<ReceiptJournalLine>, Box<dyn std::error::Error>> {
    let path = journal_path(run_dir);
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut records = Vec::new();
    for (index, line) in fs::read_to_string(&path)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            format!("receipts.jsonl line {} is invalid JSON: {error}", index + 1)
        })?;
        if value.get("format_version").is_some() {
            let envelope: SignedReceiptEnvelope =
                serde_json::from_value(value).map_err(|error| {
                    format!(
                        "receipts.jsonl line {} is not a valid signed V2 receipt: {error}",
                        index + 1
                    )
                })?;
            records.push(ReceiptJournalLine::Signed(envelope));
        } else {
            let record: ReceiptRecord = serde_json::from_value(value).map_err(|error| {
                format!(
                    "receipts.jsonl line {} is not a legacy V1 receipt: {error}",
                    index + 1
                )
            })?;
            records.push(ReceiptJournalLine::Legacy(record));
        }
    }
    Ok(records)
}

/// Parse either journal format without assigning trust. Trust-sensitive code
/// must call `load_verified_receipts` or `load_verified_receipt_journal`.
pub fn load_receipts(run_dir: &Path) -> Result<Vec<ReceiptRecord>, Box<dyn std::error::Error>> {
    Ok(load_journal_lines(run_dir)?
        .into_iter()
        .map(|line| match line {
            ReceiptJournalLine::Legacy(record) => record,
            ReceiptJournalLine::Signed(envelope) => materialize_signed_record(&envelope),
        })
        .collect())
}

fn run_id(run_dir: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let manifest: Value = serde_json::from_slice(&fs::read(run_dir.join("manifest.json"))?)?;
    manifest
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "manifest.json is missing a non-empty run_id".into())
}

fn materialize_signed_record(envelope: &SignedReceiptEnvelope) -> ReceiptRecord {
    let mut record = envelope.payload.clone();
    record.record_hash = envelope.record_hash.clone();
    record
}

/// Verify the full chain: every record's content hash matches, and every
/// prev link points at the preceding record (GENESIS for the first). Returns
/// the verified records; a broken chain is a hard error - the journal is the
/// one artifact whose integrity is non-negotiable.
pub fn load_verified_receipts(
    run_dir: &Path,
) -> Result<Vec<ReceiptRecord>, Box<dyn std::error::Error>> {
    Ok(load_verified_receipt_journal(run_dir)?.records)
}

pub fn load_verified_receipt_journal(
    run_dir: &Path,
) -> Result<VerifiedReceiptJournal, Box<dyn std::error::Error>> {
    let lines = load_journal_lines(run_dir)?;
    let expected_run_id = if lines
        .iter()
        .any(|line| matches!(line, ReceiptJournalLine::Signed(_)))
    {
        Some(run_id(run_dir)?)
    } else {
        None
    };
    let mut previous = DigestRef {
        hash_alg: "genesis".to_string(),
        digest: GENESIS.to_string(),
    };
    let mut signed_started = false;
    let mut records = Vec::new();
    let mut verification = BTreeMap::new();
    let mut last_signed_envelope = None;

    for (index, line) in lines.into_iter().enumerate() {
        let sequence = u64::try_from(index + 1)?;
        match line {
            ReceiptJournalLine::Legacy(record) => {
                if signed_started {
                    return Err(format!(
                        "receipt journal downgrade at entry {}: legacy V1 cannot follow signed V2",
                        index + 1
                    )
                    .into());
                }
                let actual = receipt_content_hash(&record);
                if record.record_hash != actual {
                    return Err(format!(
                        "receipt chain broken at entry {} ({}): record_hash {} does not match legacy content hash {} - the journal was edited after minting",
                        index + 1,
                        record.id,
                        record.record_hash,
                        actual
                    )
                    .into());
                }
                if record.prev_record_hash != previous.digest {
                    return Err(format!(
                        "receipt chain broken at entry {} ({}): prev_record_hash {} does not link to preceding record hash {}",
                        index + 1,
                        record.id,
                        record.prev_record_hash,
                        previous.digest
                    )
                    .into());
                }
                previous = DigestRef {
                    hash_alg: "fnv1a-64".to_string(),
                    digest: record.record_hash.clone(),
                };
                verification.insert(
                    record.id.clone(),
                    ReceiptVerification {
                        integrity: "legacy_weak",
                        hash_alg: "fnv1a-64",
                        principal_id: None,
                        engine: None,
                    },
                );
                records.push(record);
            }
            ReceiptJournalLine::Signed(envelope) => {
                signed_started = true;
                verify_receipt_envelope(&envelope)?;
                if envelope.run_id != expected_run_id.as_deref().unwrap_or_default() {
                    return Err(format!(
                        "signed receipt run binding mismatch at entry {}",
                        index + 1
                    )
                    .into());
                }
                if envelope.sequence != sequence || envelope.previous != previous {
                    return Err(format!(
                        "signed receipt chain broken at entry {}: sequence or typed previous digest mismatch",
                        index + 1
                    )
                    .into());
                }
                let expected_id = format!("rcpt-{sequence:04}");
                if envelope.payload.id != expected_id
                    || envelope.payload.prev_record_hash != previous.digest
                {
                    return Err(format!(
                        "signed receipt payload binding mismatch at entry {}",
                        index + 1
                    )
                    .into());
                }
                let record = materialize_signed_record(&envelope);
                verify_signed_artifacts(run_dir, &record)?;
                previous = DigestRef {
                    hash_alg: "blake3-256".to_string(),
                    digest: envelope.record_hash.clone(),
                };
                verification.insert(
                    record.id.clone(),
                    ReceiptVerification {
                        integrity: "signed",
                        hash_alg: "blake3-256",
                        principal_id: Some(envelope.executor.principal_id.clone()),
                        engine: Some(envelope.engine.clone()),
                    },
                );
                last_signed_envelope = Some(envelope);
                records.push(record);
            }
        }
    }
    verify_receipt_head(run_dir, last_signed_envelope.as_ref())?;
    Ok(VerifiedReceiptJournal {
        records,
        verification,
    })
}

fn verify_signed_artifacts(
    run_dir: &Path,
    receipt: &ReceiptRecord,
) -> Result<(), Box<dyn std::error::Error>> {
    for (stream, digest) in [
        ("stdout", receipt.stdout_hash.as_str()),
        ("stderr", receipt.stderr_hash.as_str()),
    ] {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(format!(
                "signed receipt {} has invalid BLAKE3 {stream} artifact digest {digest}",
                receipt.id
            )
            .into());
        }
        let path = run_dir
            .join("receipts")
            .join("artifacts")
            .join(format!("{digest}.txt"));
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "signed receipt {} {stream} artifact {} is missing: {error}",
                receipt.id,
                path.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "signed receipt {} {stream} artifact must be a regular non-symlink file",
                receipt.id
            )
            .into());
        }
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "signed receipt {} {stream} artifact {} is missing: {error}",
                receipt.id,
                path.display()
            )
        })?;
        let actual = blake3::hash(&bytes).to_hex().to_string();
        if actual != digest {
            return Err(format!(
                "artifact digest mismatch for receipt {} {stream}: declared {digest}, actual {actual}",
                receipt.id
            )
            .into());
        }
    }
    Ok(())
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
        let existing = load_verified_receipt_journal(run_dir)?;
        let sequence = existing.records.len() + 1;
        record.id = format!("rcpt-{sequence:04}");
        let previous = existing
            .records
            .last()
            .map(|last| -> Result<DigestRef, Box<dyn std::error::Error>> {
                let trust = existing.verification.get(&last.id).ok_or_else(|| {
                    format!("verified receipt {} is missing trust metadata", last.id)
                })?;
                Ok(DigestRef {
                    hash_alg: trust.hash_alg.to_string(),
                    digest: last.record_hash.clone(),
                })
            })
            .transpose()?
            .unwrap_or(DigestRef {
                hash_alg: "genesis".to_string(),
                digest: GENESIS.to_string(),
            });
        record.prev_record_hash = previous.digest.clone();
        record.record_hash = String::new();
        let envelope =
            sign_receipt_envelope(&run_id(run_dir)?, sequence as u64, previous, &record)?;
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        serde_json::to_writer(&mut file, &envelope)?;
        file.write_all(b"\n")?;
        file.flush()?;
        write_receipt_head(run_dir, &envelope)?;
        record.record_hash = envelope.record_hash;
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
    let hash = blake3::hash(bytes).to_hex().to_string();
    let dir = run_dir.join("receipts").join("artifacts");
    fs::create_dir_all(&dir)?;
    let rel = format!("receipts/artifacts/{hash}.txt");
    let path = run_dir.join(&rel);
    if path.exists() {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "artifact collision at {rel}: existing path is not a regular non-symlink file"
            )
            .into());
        }
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
        fs::write(
            dir.join("manifest.json"),
            format!("{{\"run_id\":\"run-{tag}-{nanos}\"}}\n"),
        )
        .expect("manifest");
        dir
    }

    fn append_blank(dir: &std::path::Path, exit_code: i64) -> ReceiptRecord {
        let mut record = blank_receipt(exit_code);
        record.stdout_hash = super::store_artifact(dir, b"hi")
            .expect("stdout artifact")
            .0;
        record.stderr_hash = super::store_artifact(dir, b"").expect("stderr artifact").0;
        append_receipt(dir, record).expect("append receipt")
    }

    #[test]
    fn chain_links_and_verifies() {
        let dir = temp_run_dir("chain");
        let first = append_blank(&dir, 0);
        let second = append_blank(&dir, 1);
        assert_eq!(first.id, "rcpt-0001");
        assert_eq!(second.prev_record_hash, first.record_hash);
        let verified = load_verified_receipts(&dir).expect("chain verifies");
        assert_eq!(verified.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn tampered_journal_breaks_the_chain() {
        let dir = temp_run_dir("tamper");
        append_blank(&dir, 0);
        let path = dir.join("receipts").join("receipts.jsonl");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("\"exit_code\":0", "\"exit_code\":1");
        fs::write(&path, text).unwrap();
        let err = load_verified_receipts(&dir).expect_err("tamper must break chain");
        assert!(
            format!("{err}").contains("record hash mismatch"),
            "got: {err}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_fnv_receipt_remains_readable_without_silent_upgrade() {
        let dir = temp_run_dir("legacy-fnv");
        let mut record = blank_receipt(0);
        record.id = "rcpt-0001".to_string();
        record.prev_record_hash = super::GENESIS.to_string();
        record.record_hash = "0000000000000000".to_string();
        record.record_hash = receipt_content_hash(&record);
        assert_eq!(record.record_hash.len(), 16, "fixture must stay legacy FNV");
        let journal = dir.join("receipts").join("receipts.jsonl");
        fs::create_dir_all(journal.parent().unwrap()).unwrap();
        fs::write(
            &journal,
            format!("{}\n", serde_json::to_string(&record).unwrap()),
        )
        .unwrap();
        let verified = load_verified_receipts(&dir).expect("legacy record remains readable");
        assert_eq!(verified, vec![record]);
        assert!(!dir.join("receipts/signed-receipts.jsonl").exists());
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
