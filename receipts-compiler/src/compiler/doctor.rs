use crate::compiler::checks::validate_manifest_bindings;
use crate::compiler::crypto::{
    BUILD_COMMIT, DEPENDENCY_LOCK_DIGEST, ENGINE_PROTOCOL_VERSION, verify_executor_key,
};
use crate::schema::{RECEIPTS_KNOWN_SCHEMA_VERSIONS, RECEIPTS_SCHEMA_VERSION};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct DoctorIdentity {
    pub protocol_version: String,
    pub engine_version: String,
    pub build_commit: String,
    pub dependency_lock_digest: String,
    pub binary_digest: String,
    pub key_fingerprint: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Serialize)]
pub struct DoctorCheck {
    pub code: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct DoctorReport {
    pub ok: bool,
    pub identity: DoctorIdentity,
    pub checks: Vec<DoctorCheck>,
}

pub fn run_doctor(repo_root: &Path) -> DoctorReport {
    let mut checks = Vec::new();
    let executable = std::env::current_exe();
    let binary_digest = executable
        .as_ref()
        .ok()
        .and_then(|path| fs::read(path).ok())
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string());
    let installation_ok = BUILD_COMMIT != "unresolved"
        && DEPENDENCY_LOCK_DIGEST.len() == 64
        && binary_digest.is_some();
    checks.push(DoctorCheck {
        code: "installation_identity".to_string(),
        status: if installation_ok { "pass" } else { "fail" }.to_string(),
        detail: if installation_ok {
            "embedded protocol, build, lock, platform, and executable digest are available"
        } else {
            "embedded installation identity is incomplete"
        }
        .to_string(),
    });

    let key = verify_executor_key();
    checks.push(DoctorCheck {
        code: "executor_key".to_string(),
        status: if key.is_ok() { "pass" } else { "fail" }.to_string(),
        detail: key
            .as_ref()
            .map(|_| "executor key is user-only and passes a sign/verify challenge".to_string())
            .unwrap_or_else(|error| error.to_string()),
    });

    let schemas_ok = RECEIPTS_SCHEMA_VERSION == "2.0.0"
        && ["1.1.0", "1.2.0", "2.0.0"]
            .iter()
            .all(|version| RECEIPTS_KNOWN_SCHEMA_VERSIONS.contains(version));
    checks.push(DoctorCheck {
        code: "schema_support".to_string(),
        status: if schemas_ok { "pass" } else { "fail" }.to_string(),
        detail: "current 2.0.0; legacy read-only 1.1.0 and 1.2.0".to_string(),
    });

    let manifest = validate_manifest_bindings(repo_root);
    checks.push(match manifest {
        Ok(Some(count)) => DoctorCheck {
            code: "check_manifest".to_string(),
            status: "pass".to_string(),
            detail: format!("{count} engine-controlled checks validated without execution"),
        },
        Ok(None) => DoctorCheck {
            code: "check_manifest".to_string(),
            status: "unavailable".to_string(),
            detail: "no .receipts/checks.toml in the current project".to_string(),
        },
        Err(error) => DoctorCheck {
            code: "check_manifest".to_string(),
            status: "fail".to_string(),
            detail: error.to_string(),
        },
    });
    checks.push(DoctorCheck {
        code: "model_runtime_metadata".to_string(),
        status: "unavailable".to_string(),
        detail: "no exact resolved model snapshot was captured; no model-specific score is allowed"
            .to_string(),
    });

    let ok = checks.iter().all(|check| check.status != "fail");
    DoctorReport {
        ok,
        identity: DoctorIdentity {
            protocol_version: ENGINE_PROTOCOL_VERSION.to_string(),
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            build_commit: BUILD_COMMIT.to_string(),
            dependency_lock_digest: DEPENDENCY_LOCK_DIGEST.to_string(),
            binary_digest: binary_digest.unwrap_or_default(),
            key_fingerprint: key.unwrap_or_default(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
        checks,
    }
}
