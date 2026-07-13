use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fresh_run(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "receipts-run-argv-{name}-{}-{nanos}",
        std::process::id()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_receipts"))
        .args(["init", dir.to_string_lossy().as_ref()])
        .output()
        .expect("init run");
    assert!(
        output.status.success(),
        "init: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    dir
}

fn core_run(run_dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_receipts"))
        .arg("run")
        .args(["--run-dir", run_dir.to_string_lossy().as_ref()])
        .args(args)
        .output()
        .expect("run receipts-core")
}

fn receipt_command(run_dir: &Path) -> Vec<String> {
    let journal = fs::read_to_string(run_dir.join("receipts/receipts.jsonl")).expect("journal");
    let row: serde_json::Value =
        serde_json::from_str(journal.lines().last().expect("receipt")).expect("receipt json");
    let receipt = row.get("payload").unwrap_or(&row);
    receipt["cmd"]
        .as_array()
        .expect("cmd array")
        .iter()
        .map(|value| value.as_str().expect("cmd token").to_string())
        .collect()
}

#[test]
fn powershell_safe_exe_and_repeated_args_preserve_exact_tokens() {
    let dir = fresh_run("exe-args");
    let output = core_run(
        &dir,
        &[
            "--label",
            "test:exe",
            "--exe",
            "rustc",
            "--arg",
            "--version",
        ],
    );
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(receipt_command(&dir), ["rustc", "--version"]);
    fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn posix_separator_form_remains_supported() {
    let dir = fresh_run("posix");
    let output = core_run(&dir, &["--", "rustc", "--version"]);
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(receipt_command(&dir), ["rustc", "--version"]);
    fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn arg_values_that_equal_parser_tokens_reach_the_child_unchanged() {
    let dir = fresh_run("literal-parser-tokens");
    let script_path = dir.join("assert-argv.cjs");
    fs::write(
        &script_path,
        "const got=process.argv.slice(2);const want=['--arg','--exe','--long-option','--'];process.exit(JSON.stringify(got)===JSON.stringify(want)?0:9);",
    )
    .expect("write child script");
    let script = script_path.to_string_lossy().to_string();
    let output = core_run(
        &dir,
        &[
            "--exe",
            "node",
            "--arg",
            &script,
            "--arg",
            "--arg",
            "--arg",
            "--exe",
            "--arg",
            "--long-option",
            "--arg",
            "--",
        ],
    );
    assert!(
        output.status.success(),
        "child rejected argv: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        receipt_command(&dir),
        vec![
            "node".to_string(),
            script,
            "--arg".to_string(),
            "--exe".to_string(),
            "--long-option".to_string(),
            "--".to_string(),
        ]
    );
    fs::remove_dir_all(dir).expect("cleanup");
}

#[test]
fn run_rejects_ambiguous_incomplete_and_guessed_command_forms() {
    let cases: &[(&str, &[&str])] = &[
        ("dual", &["--exe", "rustc", "--", "rustc", "--version"]),
        ("missing-exe", &["--exe"]),
        ("missing-arg", &["--exe", "rustc", "--arg"]),
        ("unknown", &["--exe", "rustc", "--bogus", "value"]),
        ("guess-tail", &["rustc", "--version"]),
    ];
    for (name, args) in cases {
        let dir = fresh_run(name);
        let output = core_run(&dir, args);
        assert!(!output.status.success(), "{name} unexpectedly succeeded");
        assert!(
            !dir.join("receipts/receipts.jsonl").exists(),
            "{name} minted a receipt"
        );
        fs::remove_dir_all(dir).expect("cleanup");
    }
}
