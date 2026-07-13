//! Reusable append-only signed journal for session and outcome records.

use crate::compiler::crypto::{
    DigestRef, ExecutorIdentity, SignedEngineIdentity, current_engine_identity, hex_decode,
    sign_detached, verify_detached,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const HASH_DOMAIN: &[u8] = b"agent-receipts:signed-journal:v1:record";
const SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:signed-journal:v1:signature";
const HEAD_DOMAIN: &[u8] = b"agent-receipts:signed-journal:v1:head";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SignedJournalRecord {
    pub format_version: String,
    pub record_kind: String,
    pub run_id: String,
    pub sequence: u64,
    pub previous: DigestRef,
    pub payload: Value,
    pub executor: ExecutorIdentity,
    pub engine: SignedEngineIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub record_hash: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SignedJournalHead {
    format_version: String,
    record_kind: String,
    run_id: String,
    sequence: u64,
    last: DigestRef,
    executor: ExecutorIdentity,
    signature_alg: String,
    signature: String,
}

struct JournalLock(PathBuf);

impl Drop for JournalLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn journal_path(run_dir: &Path, relative: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err("signed journal path must stay relative to the run directory".into());
    }
    Ok(run_dir.join(relative))
}

fn head_path(journal: &Path) -> PathBuf {
    journal.with_extension("head.json")
}

fn run_id(run_dir: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let manifest: Value = serde_json::from_slice(&fs::read(run_dir.join("manifest.json"))?)?;
    manifest
        .get("run_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "manifest.json is missing run_id".into())
}

fn canonical_record(record: &SignedJournalRecord) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut canonical = record.clone();
    canonical.record_hash.clear();
    canonical.signature.clear();
    Ok(serde_json::to_vec(&canonical)?)
}

fn record_hash(record: &SignedJournalRecord) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HASH_DOMAIN);
    hasher.update(&canonical_record(record)?);
    Ok(hasher.finalize().to_hex().to_string())
}

fn canonical_head(head: &SignedJournalHead) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut canonical = head.clone();
    canonical.signature.clear();
    Ok(serde_json::to_vec(&canonical)?)
}

fn acquire_lock(journal: &Path) -> Result<JournalLock, Box<dyn std::error::Error>> {
    let lock = journal.with_extension("jsonl.lock");
    for _ in 0..200 {
        match OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => return Ok(JournalLock(lock)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(format!("timed out acquiring {}", lock.display()).into())
}

fn verify_engine_shape(engine: &SignedEngineIdentity) -> Result<(), Box<dyn std::error::Error>> {
    let lower_hex = |value: &str, len: usize| {
        value.len() == len
            && value
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    };
    if engine.protocol_version.is_empty()
        || engine.engine_version.is_empty()
        || !lower_hex(&engine.build_commit, 40)
        || engine.binary_digest.hash_alg != "blake3-256"
        || !lower_hex(&engine.binary_digest.digest, 64)
        || engine.dependency_lock_digest.hash_alg != "sha256"
        || !lower_hex(&engine.dependency_lock_digest.digest, 64)
        || engine.os.is_empty()
        || engine.arch.is_empty()
    {
        return Err("signed journal engine identity is incomplete or malformed".into());
    }
    Ok(())
}

pub fn load_verified_signed_records(
    run_dir: &Path,
    relative: &str,
    record_kind: &str,
) -> Result<Vec<SignedJournalRecord>, Box<dyn std::error::Error>> {
    let journal = journal_path(run_dir, relative)?;
    let expected_run = run_id(run_dir)?;
    let mut records = Vec::new();
    if journal.exists() {
        for (index, line) in fs::read_to_string(&journal)?.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let record: SignedJournalRecord = serde_json::from_str(line).map_err(|error| {
                format!(
                    "{} line {} is not a signed record: {error}",
                    journal.display(),
                    index + 1
                )
            })?;
            records.push(record);
        }
    }
    let mut previous = DigestRef {
        hash_alg: "genesis".to_string(),
        digest: "GENESIS".to_string(),
    };
    let mut principal: Option<String> = None;
    for (index, record) in records.iter().enumerate() {
        if record.format_version != "1"
            || record.record_kind != record_kind
            || record.run_id != expected_run
            || record.sequence != (index + 1) as u64
            || record.previous != previous
            || record.hash_alg != "blake3-256"
            || record.signature_alg != "ed25519"
        {
            return Err(format!(
                "signed {record_kind} journal binding mismatch at entry {}",
                index + 1
            )
            .into());
        }
        verify_engine_shape(&record.engine)?;
        let actual = record_hash(record)?;
        if record.record_hash != actual {
            return Err(format!(
                "signed {record_kind} record hash mismatch at entry {}",
                index + 1
            )
            .into());
        }
        let digest = hex_decode(&record.record_hash)?;
        verify_detached(
            SIGNATURE_DOMAIN,
            &digest,
            &record.executor,
            &record.signature,
        )?;
        if principal
            .as_ref()
            .is_some_and(|value| value != &record.executor.principal_id)
        {
            return Err(format!(
                "signed {record_kind} journal executor continuity changed at entry {}",
                index + 1
            )
            .into());
        }
        principal = Some(record.executor.principal_id.clone());
        previous = DigestRef {
            hash_alg: "blake3-256".to_string(),
            digest: record.record_hash.clone(),
        };
    }
    verify_head(&journal, &expected_run, record_kind, records.last())?;
    Ok(records)
}

fn verify_head(
    journal: &Path,
    run_id: &str,
    record_kind: &str,
    last: Option<&SignedJournalRecord>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = head_path(journal);
    let Some(record) = last else {
        if path.exists() {
            return Err(format!("signed {record_kind} head exists without journal records").into());
        }
        return Ok(());
    };
    let head: SignedJournalHead = serde_json::from_slice(&fs::read(&path).map_err(|error| {
        format!(
            "signed {record_kind} journal is missing pinned head {}: {error}",
            path.display()
        )
    })?)?;
    if head.format_version != "1"
        || head.record_kind != format!("{record_kind}_head")
        || head.run_id != run_id
        || head.sequence != record.sequence
        || head.last.hash_alg != "blake3-256"
        || head.last.digest != record.record_hash
        || head.executor != record.executor
        || head.signature_alg != "ed25519"
    {
        return Err(
            format!("signed {record_kind} journal head does not match terminal record").into(),
        );
    }
    verify_detached(
        HEAD_DOMAIN,
        &canonical_head(&head)?,
        &head.executor,
        &head.signature,
    )
}

fn write_head(
    journal: &Path,
    record: &SignedJournalRecord,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut head = SignedJournalHead {
        format_version: "1".to_string(),
        record_kind: format!("{}_head", record.record_kind),
        run_id: record.run_id.clone(),
        sequence: record.sequence,
        last: DigestRef {
            hash_alg: "blake3-256".to_string(),
            digest: record.record_hash.clone(),
        },
        executor: record.executor.clone(),
        signature_alg: "ed25519".to_string(),
        signature: String::new(),
    };
    let (executor, signature) = sign_detached(HEAD_DOMAIN, &canonical_head(&head)?)?;
    if executor != head.executor {
        return Err("executor key changed while updating signed journal head".into());
    }
    head.signature = signature;
    let path = head_path(journal);
    let temp = path.with_extension("head.tmp");
    let mut bytes = serde_json::to_vec(&head)?;
    bytes.push(b'\n');
    fs::write(&temp, bytes)?;
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temp, path)?;
    Ok(())
}

pub fn append_signed_record(
    run_dir: &Path,
    relative: &str,
    record_kind: &str,
    payload: Value,
) -> Result<SignedJournalRecord, Box<dyn std::error::Error>> {
    let journal = journal_path(run_dir, relative)?;
    fs::create_dir_all(journal.parent().ok_or("signed journal has no parent")?)?;
    let _lock = acquire_lock(&journal)?;
    let existing = load_verified_signed_records(run_dir, relative, record_kind)?;
    let previous = existing.last().map_or(
        DigestRef {
            hash_alg: "genesis".to_string(),
            digest: "GENESIS".to_string(),
        },
        |record| DigestRef {
            hash_alg: "blake3-256".to_string(),
            digest: record.record_hash.clone(),
        },
    );
    let mut record = SignedJournalRecord {
        format_version: "1".to_string(),
        record_kind: record_kind.to_string(),
        run_id: run_id(run_dir)?,
        sequence: (existing.len() + 1) as u64,
        previous,
        payload,
        executor: ExecutorIdentity {
            principal_id: String::new(),
            public_key: String::new(),
            key_fingerprint: String::new(),
        },
        engine: current_engine_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        record_hash: String::new(),
        signature: String::new(),
    };
    // Executor identity is part of the record hash, so acquire it with a
    // harmless domain-separated signature before hashing the envelope.
    let (executor, _) = sign_detached(b"agent-receipts:signed-journal:v1:identity", b"")?;
    if existing
        .last()
        .is_some_and(|last| last.executor.principal_id != executor.principal_id)
    {
        return Err("executor key changed; refusing to fork signed journal continuity".into());
    }
    record.executor = executor;
    record.record_hash = record_hash(&record)?;
    let digest = hex_decode(&record.record_hash)?;
    let (executor, signature) = sign_detached(SIGNATURE_DOMAIN, &digest)?;
    if executor != record.executor {
        return Err("executor key changed while signing journal record".into());
    }
    record.signature = signature;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&journal)?;
    serde_json::to_writer(&mut file, &record)?;
    file.write_all(b"\n")?;
    file.flush()?;
    write_head(&journal, &record)?;
    Ok(record)
}
