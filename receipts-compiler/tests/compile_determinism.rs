//! Compile-determinism integration test.
//!
//! Guarantees that running `compile_run_dir` twice against byte-identical
//! inputs produces byte-identical `next_pass_packet.json` and `snapshot.json`
//! outputs. This is the "picture-perfect deterministic evidence" invariant —
//! a regression here means a non-deterministic code path slipped into
//! promotion, source assembly, or serialisation.

use receipts_core::compiler::run_dir::compile_run_dir;
use std::{env, fs, io, path::Path};

#[test]
fn compile_run_dir_is_byte_deterministic_across_two_runs() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/run-basic");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_a = env::temp_dir().join(format!("receipts-det-a-{pid}-{nanos}"));
    let tmp_b = env::temp_dir().join(format!("receipts-det-b-{pid}-{nanos}"));

    let _ = fs::remove_dir_all(&tmp_a);
    let _ = fs::remove_dir_all(&tmp_b);
    copy_dir_recursive(&fixture, &tmp_a).expect("copy fixture into tmp_a");
    copy_dir_recursive(&fixture, &tmp_b).expect("copy fixture into tmp_b");

    let _ = compile_run_dir(&tmp_a).expect("compile_run_dir succeeds for tmp_a");
    let _ = compile_run_dir(&tmp_b).expect("compile_run_dir succeeds for tmp_b");

    let packet_a = fs::read(tmp_a.join("state/next_pass_packet.json")).expect("read packet_a");
    let packet_b = fs::read(tmp_b.join("state/next_pass_packet.json")).expect("read packet_b");
    assert_eq!(
        packet_a, packet_b,
        "next_pass_packet.json differs between two identical compiles — a non-deterministic code path has entered the pipeline",
    );

    let snap_a = fs::read(tmp_a.join("state/snapshot.json")).expect("read snap_a");
    let snap_b = fs::read(tmp_b.join("state/snapshot.json")).expect("read snap_b");
    assert_eq!(
        snap_a, snap_b,
        "snapshot.json differs between two identical compiles — snapshot build or source collection is non-deterministic",
    );

    let log_a = fs::read(tmp_a.join("state/decision_log.jsonl")).expect("read log_a");
    let log_b = fs::read(tmp_b.join("state/decision_log.jsonl")).expect("read log_b");
    assert_eq!(
        log_a, log_b,
        "decision_log.jsonl differs between two identical compiles — decision-log source_ids ordering is non-deterministic",
    );

    let _ = fs::remove_dir_all(&tmp_a);
    let _ = fs::remove_dir_all(&tmp_b);
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
