//! Local executor identity and canonical Ed25519 receipt envelopes.
//!
//! Signatures establish continuity for records minted by the same local
//! executor key and detect post-hoc alteration. They do not prove that a
//! claim is true and do not protect a host compromised while Receipts runs.

use crate::schema::ReceiptRecord;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const SIGNATURE_FORMAT_VERSION: &str = "2";
pub const ENGINE_PROTOCOL_VERSION: &str = "1";
pub const BUILD_COMMIT: &str = env!("RECEIPTS_BUILD_COMMIT");
pub const DEPENDENCY_LOCK_DIGEST: &str = env!("RECEIPTS_LOCK_DIGEST");
const RECEIPT_HASH_DOMAIN: &[u8] = b"agent-receipts:v2:execution-receipt\0";
const RECEIPT_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:v2:ed25519-signature\0";
const RECEIPT_HEAD_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:v2:journal-head\0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutorKeyFile {
    format_version: String,
    secret_key: String,
    public_key: String,
    key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DigestRef {
    pub hash_alg: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExecutorIdentity {
    pub principal_id: String,
    pub public_key: String,
    pub key_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SignedEngineIdentity {
    pub protocol_version: String,
    pub engine_version: String,
    pub build_commit: String,
    pub binary_digest: DigestRef,
    pub dependency_lock_digest: DigestRef,
    pub os: String,
    pub arch: String,
}

/// A V2 line in receipts/receipts.jsonl. The frozen V1 ReceiptRecord remains
/// the payload so old records never need rewriting, but its record_hash is
/// blank on disk: the envelope owns the BLAKE3 digest and signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SignedReceiptEnvelope {
    pub format_version: String,
    pub record_kind: String,
    pub run_id: String,
    pub sequence: u64,
    pub previous: DigestRef,
    pub payload: ReceiptRecord,
    pub executor: ExecutorIdentity,
    pub engine: SignedEngineIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub record_hash: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct SignedReceiptHead {
    format_version: String,
    record_kind: String,
    run_id: String,
    sequence: u64,
    last: DigestRef,
    executor: ExecutorIdentity,
    signature_alg: String,
    signature: String,
}

pub fn executor_key_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    // Integration tests may isolate their key, but production/release builds
    // never accept a caller-selected executor identity.
    if cfg!(debug_assertions)
        && std::env::var_os("RECEIPTS_ALLOW_TEST_KEY_OVERRIDE").as_deref()
            == Some(std::ffi::OsStr::new("1"))
    {
        if let Some(path) = std::env::var_os("RECEIPTS_EXECUTOR_KEY") {
            return Ok(PathBuf::from(path));
        }
    }
    if cfg!(test) {
        return Ok(std::env::temp_dir()
            .join(format!("agent-receipts-test-key-{}", std::process::id()))
            .join("executor-key.json"));
    }
    #[cfg(windows)]
    {
        let base = std::env::var_os("LOCALAPPDATA")
            .ok_or("LOCALAPPDATA is unavailable; cannot protect the executor key")?;
        Ok(PathBuf::from(base)
            .join("AgentReceipts")
            .join("executor-key.json"))
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".local").join("share"))
            })
            .ok_or("HOME/XDG_DATA_HOME is unavailable; cannot protect the executor key")?;
        Ok(base.join("agent-receipts").join("executor-key.json"))
    }
}

pub fn key_fingerprint() -> Result<String, Box<dyn std::error::Error>> {
    let (_, file) = load_or_create_key()?;
    Ok(file.key_fingerprint)
}

pub fn verify_executor_key() -> Result<String, Box<dyn std::error::Error>> {
    let path = executor_key_path()?;
    let (signing, file) = load_or_create_key()?;
    let metadata = fs::symlink_metadata(&path)?;
    if metadata.file_type().is_symlink() {
        return Err("executor key must not be a symlink".into());
    }
    verify_private_permissions(&path, &metadata)?;
    let challenge = b"agent-receipts-doctor-key-challenge-v1";
    let signature = signing.sign(challenge);
    signing
        .verifying_key()
        .verify(challenge, &signature)
        .map_err(|_| "executor key sign/verify challenge failed")?;
    Ok(file.key_fingerprint)
}

#[cfg(unix)]
fn verify_private_permissions(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("executor key permissions must be user-only (0600)".into());
    }
    Ok(())
}

#[cfg(windows)]
fn verify_private_permissions(
    path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), Box<dyn std::error::Error>> {
    let principal = current_windows_principal()?;
    let acl = std::process::Command::new("icacls").arg(path).output()?;
    if !acl.status.success() {
        return Err("icacls failed while checking executor key permissions".into());
    }
    let text = String::from_utf8_lossy(&acl.stdout).to_ascii_lowercase();
    if !text.contains(&principal)
        || ["everyone", "builtin\\users", "authenticated users"]
            .iter()
            .any(|forbidden| text.contains(forbidden))
    {
        return Err("executor key ACL is not restricted to the current principal".into());
    }
    Ok(())
}

#[cfg(windows)]
fn current_windows_principal() -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("whoami").output()?;
    if !output.status.success() {
        return Err("whoami failed while protecting the executor key".into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase())
}

fn load_or_create_key() -> Result<(SigningKey, ExecutorKeyFile), Box<dyn std::error::Error>> {
    let path = executor_key_path()?;
    if path.exists() {
        return load_key(&path);
    }
    let parent = path.parent().ok_or("executor key path has no parent")?;
    fs::create_dir_all(parent)?;
    protect_key_directory(parent)?;
    let signing = SigningKey::generate(&mut OsRng);
    let public = signing.verifying_key();
    let file = ExecutorKeyFile {
        format_version: "1".to_string(),
        secret_key: hex_encode(&signing.to_bytes()),
        public_key: hex_encode(&public.to_bytes()),
        key_fingerprint: public_key_fingerprint(&public.to_bytes()),
    };
    let bytes = serde_json::to_vec_pretty(&file)?;
    let create_result = create_private_file(&path, &bytes);
    match create_result {
        Ok(()) => load_key(&path),
        Err(error) if path.exists() => load_key(&path).map_err(|_| error),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn protect_key_directory(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn protect_key_directory(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let principal = current_windows_principal()?;
    let grant = format!("{principal}:(OI)(CI)(F)");
    let output = std::process::Command::new("icacls")
        .arg(path)
        .args(["/inheritance:r", "/grant:r", &grant])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "icacls failed while protecting executor key directory: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

fn load_key(path: &Path) -> Result<(SigningKey, ExecutorKeyFile), Box<dyn std::error::Error>> {
    let file: ExecutorKeyFile = serde_json::from_slice(&fs::read(path)?)?;
    if file.format_version != "1" {
        return Err("unsupported executor key format".into());
    }
    let secret: [u8; 32] = hex_decode(&file.secret_key)?
        .try_into()
        .map_err(|_| "executor secret key must contain 32 bytes")?;
    let signing = SigningKey::from_bytes(&secret);
    let public = signing.verifying_key();
    let expected_public = hex_encode(&public.to_bytes());
    let expected_fingerprint = public_key_fingerprint(&public.to_bytes());
    if file.public_key != expected_public || file.key_fingerprint != expected_fingerprint {
        return Err("executor key file public identity does not match its secret key".into());
    }
    Ok((signing, file))
}

#[cfg(unix)]
fn create_private_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

#[cfg(windows)]
fn create_private_file(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    // The parent DACL is restricted before creation, so the new key never
    // inherits a broad ACL and is then repaired after exposure.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn public_key_fingerprint(bytes: &[u8; 32]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn current_binary_digest() -> Result<String, Box<dyn std::error::Error>> {
    Ok(blake3::hash(&fs::read(std::env::current_exe()?)?)
        .to_hex()
        .to_string())
}

fn canonical_unsigned_envelope(
    envelope: &SignedReceiptEnvelope,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut canonical = envelope.clone();
    canonical.record_hash.clear();
    canonical.signature.clear();
    Ok(serde_json::to_vec(&canonical)?)
}

fn envelope_hash(envelope: &SignedReceiptEnvelope) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(RECEIPT_HASH_DOMAIN);
    hasher.update(&canonical_unsigned_envelope(envelope)?);
    Ok(hasher.finalize().to_hex().to_string())
}

fn signature_message(record_hash: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let digest = hex_decode(record_hash)?;
    if digest.len() != 32 {
        return Err("signed receipt record hash must contain 32 bytes".into());
    }
    let mut message = Vec::with_capacity(RECEIPT_SIGNATURE_DOMAIN.len() + digest.len());
    message.extend_from_slice(RECEIPT_SIGNATURE_DOMAIN);
    message.extend_from_slice(&digest);
    Ok(message)
}

pub fn sign_receipt_envelope(
    run_id: &str,
    sequence: u64,
    previous: DigestRef,
    payload: &ReceiptRecord,
) -> Result<SignedReceiptEnvelope, Box<dyn std::error::Error>> {
    if !cfg!(test)
        && (BUILD_COMMIT.len() != 40
            || !BUILD_COMMIT.bytes().all(|byte| byte.is_ascii_hexdigit())
            || DEPENDENCY_LOCK_DIGEST.len() != 64
            || !DEPENDENCY_LOCK_DIGEST
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err("refusing to sign with unresolved build or dependency-lock identity".into());
    }
    if payload.record_hash != "" {
        return Err("signed receipt payload record_hash must be blank".into());
    }
    let (signing, key) = load_or_create_key()?;
    let mut envelope = SignedReceiptEnvelope {
        format_version: SIGNATURE_FORMAT_VERSION.to_string(),
        record_kind: "execution_receipt".to_string(),
        run_id: run_id.to_string(),
        sequence,
        previous,
        payload: payload.clone(),
        executor: ExecutorIdentity {
            principal_id: format!("ed25519:{}", key.key_fingerprint),
            public_key: key.public_key,
            key_fingerprint: key.key_fingerprint,
        },
        engine: SignedEngineIdentity {
            protocol_version: ENGINE_PROTOCOL_VERSION.to_string(),
            engine_version: env!("CARGO_PKG_VERSION").to_string(),
            build_commit: BUILD_COMMIT.to_string(),
            binary_digest: DigestRef {
                hash_alg: "blake3-256".to_string(),
                digest: current_binary_digest()?,
            },
            dependency_lock_digest: DigestRef {
                hash_alg: "sha256".to_string(),
                digest: DEPENDENCY_LOCK_DIGEST.to_string(),
            },
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        },
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        record_hash: String::new(),
        signature: String::new(),
    };
    envelope.record_hash = envelope_hash(&envelope)?;
    envelope.signature = hex_encode(
        &signing
            .sign(&signature_message(&envelope.record_hash)?)
            .to_bytes(),
    );
    Ok(envelope)
}

pub fn verify_receipt_envelope(
    envelope: &SignedReceiptEnvelope,
) -> Result<(), Box<dyn std::error::Error>> {
    if envelope.format_version != SIGNATURE_FORMAT_VERSION
        || envelope.record_kind != "execution_receipt"
        || envelope.hash_alg != "blake3-256"
        || envelope.signature_alg != "ed25519"
        || envelope.engine.protocol_version != ENGINE_PROTOCOL_VERSION
        || envelope.engine.binary_digest.hash_alg != "blake3-256"
        || envelope.engine.dependency_lock_digest.hash_alg != "sha256"
        || envelope.payload.record_hash != ""
        || !lower_hex(&envelope.record_hash, 64)
        || !lower_hex(&envelope.signature, 128)
        || !lower_hex(&envelope.executor.public_key, 64)
        || !lower_hex(&envelope.executor.key_fingerprint, 64)
        || !lower_hex(&envelope.engine.build_commit, 40)
        || !lower_hex(&envelope.engine.binary_digest.digest, 64)
        || !lower_hex(&envelope.engine.dependency_lock_digest.digest, 64)
        || envelope.run_id.is_empty()
        || envelope.run_id.len() > 200
        || envelope.engine.engine_version.is_empty()
        || envelope.engine.engine_version.len() > 64
        || envelope.engine.os.is_empty()
        || envelope.engine.os.len() > 32
        || envelope.engine.arch.is_empty()
        || envelope.engine.arch.len() > 32
    {
        return Err("signed receipt envelope has unsupported or incoherent metadata".into());
    }
    let public_bytes: [u8; 32] = hex_decode(&envelope.executor.public_key)?
        .try_into()
        .map_err(|_| "signed receipt public key must contain 32 bytes")?;
    let public = VerifyingKey::from_bytes(&public_bytes)?;
    let expected_fingerprint = public_key_fingerprint(&public_bytes);
    if envelope.executor.key_fingerprint != expected_fingerprint
        || envelope.executor.principal_id != format!("ed25519:{expected_fingerprint}")
    {
        return Err("signed receipt public-key fingerprint mismatch".into());
    }
    let actual_hash = envelope_hash(envelope)?;
    if envelope.record_hash != actual_hash {
        return Err(format!(
            "signed receipt record hash mismatch: declared {}, actual {actual_hash}",
            envelope.record_hash
        )
        .into());
    }
    let signature_bytes: [u8; 64] = hex_decode(&envelope.signature)?
        .try_into()
        .map_err(|_| "receipt signature must contain 64 bytes")?;
    public
        .verify(
            &signature_message(&envelope.record_hash)?,
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_| "receipt signature verification failed".into())
}

fn lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn receipt_head_path(run_dir: &Path) -> PathBuf {
    run_dir.join("receipts").join("signed-head.json")
}

fn canonical_unsigned_head(
    head: &SignedReceiptHead,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut canonical = head.clone();
    canonical.signature.clear();
    Ok(serde_json::to_vec(&canonical)?)
}

fn head_signature_message(head: &SignedReceiptHead) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let canonical = canonical_unsigned_head(head)?;
    let mut message = Vec::with_capacity(RECEIPT_HEAD_SIGNATURE_DOMAIN.len() + canonical.len());
    message.extend_from_slice(RECEIPT_HEAD_SIGNATURE_DOMAIN);
    message.extend_from_slice(&canonical);
    Ok(message)
}

pub fn write_receipt_head(
    run_dir: &Path,
    envelope: &SignedReceiptEnvelope,
) -> Result<(), Box<dyn std::error::Error>> {
    let (signing, key) = load_or_create_key()?;
    if key.key_fingerprint != envelope.executor.key_fingerprint {
        return Err("executor key changed while updating signed receipt head".into());
    }
    let mut head = SignedReceiptHead {
        format_version: SIGNATURE_FORMAT_VERSION.to_string(),
        record_kind: "execution_receipt_head".to_string(),
        run_id: envelope.run_id.clone(),
        sequence: envelope.sequence,
        last: DigestRef {
            hash_alg: envelope.hash_alg.clone(),
            digest: envelope.record_hash.clone(),
        },
        executor: envelope.executor.clone(),
        signature_alg: "ed25519".to_string(),
        signature: String::new(),
    };
    head.signature = hex_encode(&signing.sign(&head_signature_message(&head)?).to_bytes());
    let path = receipt_head_path(run_dir);
    fs::create_dir_all(path.parent().ok_or("receipt head has no parent")?)?;
    let temp = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temp)?;
    serde_json::to_writer(&mut file, &head)?;
    file.write_all(b"\n")?;
    file.flush()?;
    drop(file);
    if path.exists() {
        fs::remove_file(&path)?;
    }
    fs::rename(temp, path)?;
    Ok(())
}

pub fn verify_receipt_head(
    run_dir: &Path,
    last_envelope: Option<&SignedReceiptEnvelope>,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = receipt_head_path(run_dir);
    let Some(envelope) = last_envelope else {
        if path.exists() {
            return Err("signed receipt head exists without a signed journal record".into());
        }
        return Ok(());
    };
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "signed receipt journal is missing its pinned head {}: {error}",
            path.display()
        )
    })?;
    let head: SignedReceiptHead = serde_json::from_slice(&bytes)
        .map_err(|error| format!("signed receipt head is invalid: {error}"))?;
    if head.format_version != SIGNATURE_FORMAT_VERSION
        || head.record_kind != "execution_receipt_head"
        || head.run_id != envelope.run_id
        || head.sequence != envelope.sequence
        || head.last.hash_alg != envelope.hash_alg
        || head.last.digest != envelope.record_hash
        || head.executor != envelope.executor
        || head.signature_alg != "ed25519"
    {
        return Err("signed receipt journal head does not match the terminal record".into());
    }
    let public_bytes: [u8; 32] = hex_decode(&head.executor.public_key)?
        .try_into()
        .map_err(|_| "signed receipt head public key must contain 32 bytes")?;
    let public = VerifyingKey::from_bytes(&public_bytes)?;
    let signature_bytes: [u8; 64] = hex_decode(&head.signature)?
        .try_into()
        .map_err(|_| "receipt head signature must contain 64 bytes")?;
    public
        .verify(
            &head_signature_message(&head)?,
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_| "receipt head signature verification failed".into())
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub fn hex_decode(value: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    if value.len() % 2 != 0
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("invalid lowercase hex".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| Ok(u8::from_str_radix(&value[index..index + 2], 16)?))
        .collect()
}
