mod build_support;

use build_support::{cargo_vcs_head, is_hex};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=RECEIPTS_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=RECEIPTS_LOCK_DIGEST");
    println!("cargo:rerun-if-changed=Cargo.lock");
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let vcs_info = manifest_dir.join(".cargo_vcs_info.json");
    if vcs_info.exists() {
        println!("cargo:rerun-if-changed=.cargo_vcs_info.json");
    }
    let repo_root = manifest_dir.parent().unwrap_or(&manifest_dir);
    let build_commit = std::env::var("RECEIPTS_BUILD_COMMIT")
        .ok()
        .filter(|value| is_hex(value, 40))
        .or_else(|| git_head(repo_root))
        .or_else(|| cargo_vcs_head(&vcs_info))
        .unwrap_or_else(|| "unresolved".to_string());
    let lock_digest = std::env::var("RECEIPTS_LOCK_DIGEST")
        .ok()
        .filter(|value| is_hex(value, 64))
        .or_else(|| sha256_file(&manifest_dir.join("Cargo.lock")))
        .unwrap_or_else(|| "unresolved".to_string());
    println!("cargo:rustc-env=RECEIPTS_BUILD_COMMIT={build_commit}");
    println!("cargo:rustc-env=RECEIPTS_LOCK_DIGEST={lock_digest}");
}

fn git_head(repo_root: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["-C", repo_root.to_str()?, "rev-parse", "HEAD"])
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    (output.status.success() && is_hex(&value, 40)).then_some(value)
}

fn sha256_file(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let digest = Sha256::digest(bytes);
    Some(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}
