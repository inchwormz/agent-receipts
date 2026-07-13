//! F5 regression: `receipts init` must scaffold a run the rest of the pipeline
//! can actually finish. The original Rust init seeded `vf-synthesis-pending`
//! while the JS synthesis recorder only consumes `vf-codex-synthesis-pending`,
//! so every Rust-scaffolded run was permanently gate-red. This test pins the
//! shared contract: seed finding id, typed finding_kind, and repo_root in the
//! manifest (F3).

use std::fs;
use std::process::Command;

#[test]
fn init_scaffolds_a_finishable_run() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "receipts-init-contract-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);

    let repo_root = std::env::temp_dir().join(format!("receipts-init-root-{nanos}"));
    fs::create_dir_all(&repo_root).expect("create fake repo root");

    let output = Command::new(env!("CARGO_BIN_EXE_receipts"))
        .arg("init")
        .arg(&dir)
        .arg("--repo-root")
        .arg(&repo_root)
        .output()
        .expect("run receipts-core init");
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).expect("manifest"))
            .expect("manifest parses");
    assert_eq!(
        manifest["repo_root"].as_str(),
        Some(repo_root.to_string_lossy().as_ref()),
        "init must record repo_root in the manifest (F3)"
    );

    let findings =
        fs::read_to_string(dir.join("verifier-results/findings.jsonl")).expect("findings");
    let seed: serde_json::Value = serde_json::from_str(findings.lines().next().expect("seed line"))
        .expect("seed finding parses");
    assert_eq!(
        seed["id"].as_str(),
        Some("vf-codex-synthesis-pending"),
        "seed finding id must match what the synthesis recorder consumes (F5)"
    );
    assert_eq!(
        seed["finding_kind"].as_str(),
        Some("synthesis"),
        "seed finding must carry a typed finding_kind (F6)"
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&repo_root);
}
