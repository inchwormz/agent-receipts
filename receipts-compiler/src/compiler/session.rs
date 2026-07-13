//! Exact model/agent session capture. Adapters inspect explicit local metadata
//! and environment fields; they never turn a mutable alias into a snapshot.

use crate::compiler::crypto::{SignedEngineIdentity, current_engine_identity};
use crate::compiler::signed_journal::{
    SignedJournalRecord, append_signed_record, load_verified_signed_records,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const JOURNAL: &str = "sessions/sessions.jsonl";
const KIND: &str = "session_capture";

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct AdapterMetadata {
    provider: Option<String>,
    requested_model: Option<String>,
    resolved_model_snapshot: Option<String>,
    provider_request_id: Option<String>,
    provider_session_id: Option<String>,
    agent_name: Option<String>,
    agent_version: Option<String>,
    scaffold_name: Option<String>,
    scaffold_version: Option<String>,
    tool_configuration: Option<Value>,
    reasoning_setting: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionCapture {
    pub session_capture_id: String,
    pub captured_at: String,
    pub adapter: String,
    pub provider: Option<String>,
    pub requested_model: Option<String>,
    pub resolved_model_snapshot: Option<String>,
    pub resolution_status: String,
    pub model_specific_eligible: bool,
    pub provider_request_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub scaffold_name: Option<String>,
    pub scaffold_version: Option<String>,
    pub tool_configuration_digest: Option<String>,
    pub reasoning_setting_digest: Option<String>,
    pub engine: SignedEngineIdentity,
    pub environment_digest: String,
    pub metadata_sources: Vec<String>,
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_value(names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .and_then(|value| nonempty(Some(value)))
}

fn digest_value(value: Option<Value>) -> Result<Option<String>, Box<dyn std::error::Error>> {
    value
        .map(|value| {
            Ok(blake3::hash(&serde_json::to_vec(&value)?)
                .to_hex()
                .to_string())
        })
        .transpose()
}

fn read_adapter_metadata(
    run_dir: &Path,
    adapter: &str,
) -> Result<(AdapterMetadata, Vec<String>), Box<dyn std::error::Error>> {
    let path = run_dir.join("session").join(format!("{adapter}.json"));
    if path.exists() {
        let metadata: AdapterMetadata = serde_json::from_slice(&fs::read(&path)?)?;
        return Ok((metadata, vec![format!("session/{adapter}.json")]));
    }
    let metadata = match adapter {
        "codex" => AdapterMetadata {
            provider: Some("openai".to_string()),
            requested_model: env_value(&["CODEX_MODEL", "OPENAI_MODEL"]),
            resolved_model_snapshot: env_value(&["CODEX_RESOLVED_MODEL_SNAPSHOT"]),
            provider_request_id: env_value(&["OPENAI_REQUEST_ID"]),
            provider_session_id: env_value(&["CODEX_SESSION_ID"]),
            agent_name: Some("codex".to_string()),
            agent_version: env_value(&["CODEX_VERSION"]),
            scaffold_name: env_value(&["CODEX_SCAFFOLD_NAME"]),
            scaffold_version: env_value(&["CODEX_SCAFFOLD_VERSION"]),
            tool_configuration: env_value(&["CODEX_TOOL_CONFIGURATION_DIGEST_INPUT"])
                .map(Value::String),
            reasoning_setting: env_value(&["CODEX_REASONING_SETTING"]).map(Value::String),
        },
        "claude" => AdapterMetadata {
            provider: Some("anthropic".to_string()),
            requested_model: env_value(&["ANTHROPIC_MODEL", "CLAUDE_MODEL"]),
            resolved_model_snapshot: env_value(&["CLAUDE_RESOLVED_MODEL_SNAPSHOT"]),
            provider_request_id: env_value(&["ANTHROPIC_REQUEST_ID"]),
            provider_session_id: env_value(&["CLAUDE_CODE_SESSION_ID"]),
            agent_name: Some("claude-code".to_string()),
            agent_version: env_value(&["CLAUDE_CODE_VERSION"]),
            scaffold_name: env_value(&["CLAUDE_SCAFFOLD_NAME"]),
            scaffold_version: env_value(&["CLAUDE_SCAFFOLD_VERSION"]),
            tool_configuration: env_value(&["CLAUDE_TOOL_CONFIGURATION_DIGEST_INPUT"])
                .map(Value::String),
            reasoning_setting: env_value(&["CLAUDE_REASONING_SETTING"]).map(Value::String),
        },
        "generic" => AdapterMetadata::default(),
        _ => return Err(format!("unsupported session adapter `{adapter}`").into()),
    };
    Ok((metadata, vec!["process-environment:allowlist".to_string()]))
}

fn environment_digest(engine: &SignedEngineIdentity) -> Result<String, Box<dyn std::error::Error>> {
    let mut environment = BTreeMap::new();
    environment.insert("os", engine.os.clone());
    environment.insert("arch", engine.arch.clone());
    environment.insert("binary", engine.binary_digest.digest.clone());
    environment.insert("lock", engine.dependency_lock_digest.digest.clone());
    for name in ["CI", "ComSpec", "SHELL", "TERM_PROGRAM"] {
        if let Ok(value) = std::env::var(name) {
            environment.insert(name, value);
        }
    }
    Ok(blake3::hash(&serde_json::to_vec(&environment)?)
        .to_hex()
        .to_string())
}

pub fn capture_session(
    run_dir: &Path,
    adapter: &str,
    captured_at: &str,
) -> Result<SignedJournalRecord, Box<dyn std::error::Error>> {
    let (metadata, sources) = read_adapter_metadata(run_dir, adapter)?;
    let engine = current_engine_identity()?;
    let resolved = nonempty(metadata.resolved_model_snapshot);
    let provider = nonempty(metadata.provider);
    let agent_name = nonempty(metadata.agent_name);
    let agent_version = nonempty(metadata.agent_version);
    let model_specific_eligible =
        resolved.is_some() && provider.is_some() && agent_name.is_some() && agent_version.is_some();
    let payload = SessionCapture {
        session_capture_id: format!(
            "session-{}",
            &blake3::hash(
                format!(
                    "{adapter}:{captured_at}:{}",
                    metadata.provider_session_id.as_deref().unwrap_or("")
                )
                .as_bytes()
            )
            .to_hex()[..16]
        ),
        captured_at: captured_at.to_string(),
        adapter: adapter.to_string(),
        provider,
        requested_model: nonempty(metadata.requested_model),
        resolution_status: if resolved.is_some() {
            "resolved".to_string()
        } else {
            "unresolved".to_string()
        },
        model_specific_eligible,
        resolved_model_snapshot: resolved,
        provider_request_id: nonempty(metadata.provider_request_id),
        provider_session_id: nonempty(metadata.provider_session_id),
        agent_name,
        agent_version,
        scaffold_name: nonempty(metadata.scaffold_name),
        scaffold_version: nonempty(metadata.scaffold_version),
        tool_configuration_digest: digest_value(metadata.tool_configuration)?,
        reasoning_setting_digest: digest_value(metadata.reasoning_setting)?,
        environment_digest: environment_digest(&engine)?,
        engine,
        metadata_sources: sources,
    };
    append_signed_record(run_dir, JOURNAL, KIND, serde_json::to_value(payload)?)
}

pub fn load_sessions(run_dir: &Path) -> Result<Vec<SessionCapture>, Box<dyn std::error::Error>> {
    load_verified_signed_records(run_dir, JOURNAL, KIND)?
        .into_iter()
        .map(|record| Ok(serde_json::from_value(record.payload)?))
        .collect()
}

pub fn latest_session(run_dir: &Path) -> Result<SessionCapture, Box<dyn std::error::Error>> {
    load_sessions(run_dir)?
        .pop()
        .ok_or_else(|| "no captured session; run `receipts session capture` first".into())
}
