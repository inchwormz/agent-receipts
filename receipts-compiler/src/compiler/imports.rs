//! Strict, local import of pinned external evaluation files.
//!
//! The original bytes stay local. The signed descriptor records provenance
//! and fixed statistical weight; it does not turn vendor or model-card claims
//! into independently adjudicated outcomes.

use crate::compiler::crypto::{
    ExecutorIdentity, SignedEngineIdentity, current_engine_identity, sign_detached, verify_detached,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const IMPORT_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:v1:eval-import";
const IMPORT_HASH_DOMAIN: &[u8] = b"agent-receipts:v1:eval-import-hash\0";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedEval {
    format_version: String,
    data_kind: String,
    source_url: String,
    retrieval_date: String,
    methodology_version: String,
    harness_version: String,
    sample_size: u64,
    attribution: String,
    license: String,
    records: Vec<EvalRecord>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalRecord {
    task_id: String,
    result: String,
    provider: String,
    model_snapshot: String,
    agent_name: String,
    agent_version: String,
    task_family: String,
    repository_id: String,
    language: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedTaskResult {
    pub dataset_hash: String,
    pub task_id: String,
    pub result: String,
    pub provider: String,
    pub model_snapshot: String,
    pub agent_name: String,
    pub agent_version: String,
    pub task_family: String,
    pub repository_id: String,
    pub language: String,
    pub weight: f64,
}

#[derive(Debug, Clone)]
pub struct VerifiedImports {
    pub source_dataset_hashes: Vec<String>,
    pub task_results: Vec<ImportedTaskResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalImportReceipt {
    pub format_version: String,
    pub record_kind: String,
    pub dataset_hash_alg: String,
    pub dataset_hash: String,
    pub data_kind: String,
    pub source_url: String,
    pub retrieval_date: String,
    pub methodology_version: String,
    pub harness_version: String,
    pub sample_size: u64,
    pub attribution: String,
    pub license: String,
    pub prior_weight: f64,
    pub source_byte_length: u64,
    pub imported_at: String,
    pub executor: ExecutorIdentity,
    pub engine: SignedEngineIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub receipt_hash: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalImportReport {
    pub ok: bool,
    pub dataset_hash: String,
    pub data_kind: String,
    pub sample_size: u64,
    pub prior_weight: f64,
    pub stored_file: PathBuf,
    pub receipt_file: PathBuf,
}

fn required(label: &str, value: &str) -> Result<(), Box<dyn std::error::Error>> {
    if value.trim().is_empty() || value.len() > 500 {
        return Err(format!("pinned evaluation {label} must be non-empty and bounded").into());
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
        && value[5..7]
            .parse::<u8>()
            .is_ok_and(|month| (1..=12).contains(&month))
        && value[8..10]
            .parse::<u8>()
            .is_ok_and(|day| (1..=31).contains(&day))
}

fn validate(eval: &PinnedEval) -> Result<f64, Box<dyn std::error::Error>> {
    if eval.format_version != "1" {
        return Err("unsupported pinned evaluation format_version; expected `1`".into());
    }
    let prior_weight = match eval.data_kind.as_str() {
        "task-results" => 0.25,
        "model-card-metadata" => 0.0,
        _ => return Err("data_kind must be `task-results` or `model-card-metadata`".into()),
    };
    if !eval.source_url.starts_with("https://") {
        return Err("pinned evaluation source_url must use https".into());
    }
    if !valid_date(&eval.retrieval_date) {
        return Err("pinned evaluation retrieval_date must be YYYY-MM-DD".into());
    }
    for (label, value) in [
        ("source_url", eval.source_url.as_str()),
        ("methodology_version", eval.methodology_version.as_str()),
        ("harness_version", eval.harness_version.as_str()),
        ("attribution", eval.attribution.as_str()),
        ("license", eval.license.as_str()),
    ] {
        required(label, value)?;
    }
    if eval.sample_size == 0 || eval.sample_size as usize != eval.records.len() {
        return Err("sample_size must be positive and exactly match records.length".into());
    }
    let mut task_ids = HashSet::new();
    for record in &eval.records {
        for (label, value) in [
            ("task_id", record.task_id.as_str()),
            ("provider", record.provider.as_str()),
            ("model_snapshot", record.model_snapshot.as_str()),
            ("agent_name", record.agent_name.as_str()),
            ("agent_version", record.agent_version.as_str()),
            ("task_family", record.task_family.as_str()),
            ("repository_id", record.repository_id.as_str()),
            ("language", record.language.as_str()),
        ] {
            required(label, value)?;
        }
        if !matches!(record.result.as_str(), "success" | "failure") {
            return Err("each imported task result must be `success` or `failure`".into());
        }
        if !task_ids.insert(&record.task_id) {
            return Err(format!("duplicate imported task_id `{}`", record.task_id).into());
        }
    }
    Ok(prior_weight)
}

fn canonical_unsigned(receipt: &EvalImportReceipt) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut unsigned = receipt.clone();
    unsigned.receipt_hash.clear();
    unsigned.signature.clear();
    Ok(serde_json::to_vec(&unsigned)?)
}

fn receipt_hash(receipt: &EvalImportReceipt) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(IMPORT_HASH_DOMAIN);
    hasher.update(&canonical_unsigned(receipt)?);
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn verify_import_receipt(
    receipt: &EvalImportReceipt,
) -> Result<(), Box<dyn std::error::Error>> {
    if receipt.format_version != "1"
        || receipt.record_kind != "eval_import"
        || receipt.dataset_hash_alg != "blake3-256"
        || receipt.hash_alg != "blake3-256"
        || receipt.signature_alg != "ed25519"
        || receipt.dataset_hash.len() != 64
        || receipt.receipt_hash.len() != 64
    {
        return Err("evaluation import receipt has unsupported metadata".into());
    }
    let actual = receipt_hash(receipt)?;
    if receipt.receipt_hash != actual {
        return Err("evaluation import receipt hash mismatch".into());
    }
    verify_detached(
        IMPORT_SIGNATURE_DOMAIN,
        receipt.receipt_hash.as_bytes(),
        &receipt.executor,
        &receipt.signature,
    )
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    Ok(())
}

pub fn import_eval(
    source: &Path,
    out_dir: &Path,
    imported_at: &str,
) -> Result<EvalImportReport, Box<dyn std::error::Error>> {
    if !source.is_file() {
        return Err(format!("pinned evaluation file not found: {}", source.display()).into());
    }
    let source_bytes = fs::read(source)?;
    let eval: PinnedEval = serde_json::from_slice(&source_bytes)?;
    let prior_weight = validate(&eval)?;
    let dataset_hash = blake3::hash(&source_bytes).to_hex().to_string();
    fs::create_dir_all(out_dir)?;
    let stored_file = out_dir.join(format!("{dataset_hash}.json"));
    let receipt_file = out_dir.join(format!("{dataset_hash}.receipt.json"));
    if stored_file.exists() || receipt_file.exists() {
        return Err(format!("dataset {dataset_hash} was already imported").into());
    }

    let mut receipt = EvalImportReceipt {
        format_version: "1".to_string(),
        record_kind: "eval_import".to_string(),
        dataset_hash_alg: "blake3-256".to_string(),
        dataset_hash: dataset_hash.clone(),
        data_kind: eval.data_kind.clone(),
        source_url: eval.source_url,
        retrieval_date: eval.retrieval_date,
        methodology_version: eval.methodology_version,
        harness_version: eval.harness_version,
        sample_size: eval.sample_size,
        attribution: eval.attribution,
        license: eval.license,
        prior_weight,
        source_byte_length: source_bytes.len() as u64,
        imported_at: imported_at.to_string(),
        executor: crate::compiler::crypto::current_executor_identity()?,
        engine: current_engine_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        receipt_hash: String::new(),
        signature: String::new(),
    };
    receipt.receipt_hash = receipt_hash(&receipt)?;
    let (executor, signature) =
        sign_detached(IMPORT_SIGNATURE_DOMAIN, receipt.receipt_hash.as_bytes())?;
    if executor != receipt.executor {
        return Err("executor identity changed while signing evaluation import".into());
    }
    receipt.signature = signature;
    verify_import_receipt(&receipt)?;

    let mut receipt_bytes = serde_json::to_vec_pretty(&receipt)?;
    receipt_bytes.push(b'\n');
    write_new(&stored_file, &source_bytes)?;
    if let Err(error) = write_new(&receipt_file, &receipt_bytes) {
        let _ = fs::remove_file(&stored_file);
        return Err(error);
    }
    Ok(EvalImportReport {
        ok: true,
        dataset_hash,
        data_kind: eval.data_kind,
        sample_size: eval.sample_size,
        prior_weight,
        stored_file,
        receipt_file,
    })
}

pub fn load_verified_imports(
    directory: &Path,
) -> Result<VerifiedImports, Box<dyn std::error::Error>> {
    if !directory.is_dir() {
        return Err(format!(
            "calibration imports directory not found: {}",
            directory.display()
        )
        .into());
    }
    let mut receipts: Vec<PathBuf> = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".receipt.json"))
        })
        .collect();
    receipts.sort();
    let mut source_dataset_hashes = Vec::new();
    let mut task_results = Vec::new();
    for receipt_path in receipts {
        let receipt: EvalImportReceipt = serde_json::from_slice(&fs::read(&receipt_path)?)?;
        verify_import_receipt(&receipt)?;
        let expected_name = format!("{}.receipt.json", receipt.dataset_hash);
        if receipt_path.file_name().and_then(|name| name.to_str()) != Some(&expected_name) {
            return Err(
                "evaluation import receipt filename does not match its dataset hash".into(),
            );
        }
        let source_path = directory.join(format!("{}.json", receipt.dataset_hash));
        let source_bytes = fs::read(&source_path).map_err(|error| {
            format!(
                "evaluation import source is missing for {}: {error}",
                receipt.dataset_hash
            )
        })?;
        let actual_hash = blake3::hash(&source_bytes).to_hex().to_string();
        if actual_hash != receipt.dataset_hash
            || source_bytes.len() as u64 != receipt.source_byte_length
        {
            return Err(format!(
                "evaluation import source hash/length mismatch for {}",
                receipt.dataset_hash
            )
            .into());
        }
        let eval: PinnedEval = serde_json::from_slice(&source_bytes)?;
        let weight = validate(&eval)?;
        if eval.data_kind != receipt.data_kind
            || eval.source_url != receipt.source_url
            || eval.retrieval_date != receipt.retrieval_date
            || eval.methodology_version != receipt.methodology_version
            || eval.harness_version != receipt.harness_version
            || eval.sample_size != receipt.sample_size
            || eval.attribution != receipt.attribution
            || eval.license != receipt.license
            || weight != receipt.prior_weight
        {
            return Err("evaluation import receipt provenance disagrees with source bytes".into());
        }
        source_dataset_hashes.push(receipt.dataset_hash.clone());
        if eval.data_kind == "task-results" {
            task_results.extend(eval.records.into_iter().map(|record| ImportedTaskResult {
                dataset_hash: receipt.dataset_hash.clone(),
                task_id: record.task_id,
                result: record.result,
                provider: record.provider,
                model_snapshot: record.model_snapshot,
                agent_name: record.agent_name,
                agent_version: record.agent_version,
                task_family: record.task_family,
                repository_id: record.repository_id,
                language: record.language,
                weight,
            }));
        }
    }
    source_dataset_hashes.sort();
    source_dataset_hashes.dedup();
    task_results.sort_by(|left, right| {
        (&left.dataset_hash, &left.task_id).cmp(&(&right.dataset_hash, &right.task_id))
    });
    Ok(VerifiedImports {
        source_dataset_hashes,
        task_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_receipt_fails_verification() {
        let mut receipt = EvalImportReceipt {
            format_version: "1".into(),
            record_kind: "eval_import".into(),
            dataset_hash_alg: "blake3-256".into(),
            dataset_hash: "a".repeat(64),
            data_kind: "task-results".into(),
            source_url: "https://example.invalid/eval".into(),
            retrieval_date: "2026-07-14".into(),
            methodology_version: "m1".into(),
            harness_version: "h1".into(),
            sample_size: 1,
            attribution: "authors".into(),
            license: "CC-BY-4.0".into(),
            prior_weight: 0.25,
            source_byte_length: 1,
            imported_at: "2026-07-14T00:00:00Z".into(),
            executor: crate::compiler::crypto::current_executor_identity().unwrap(),
            engine: current_engine_identity().unwrap(),
            hash_alg: "blake3-256".into(),
            signature_alg: "ed25519".into(),
            receipt_hash: String::new(),
            signature: String::new(),
        };
        receipt.receipt_hash = receipt_hash(&receipt).unwrap();
        let (_, signature) =
            sign_detached(IMPORT_SIGNATURE_DOMAIN, receipt.receipt_hash.as_bytes()).unwrap();
        receipt.signature = signature;
        verify_import_receipt(&receipt).unwrap();
        receipt.sample_size = 2;
        assert!(verify_import_receipt(&receipt).is_err());
    }
}
