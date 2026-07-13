use receipts_core::compiler::receipts::{
    WORK_LABEL, append_receipt, git_tree_state, store_artifact,
};
use receipts_core::compiler::report::generate_report;
use receipts_core::compiler::run_dir::compile_run_dir;
use receipts_core::schema::ReceiptRecord;
use std::fs;
use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const ENGINE_PROTOCOL_VERSION: &str = "1";
const BUILD_COMMIT: &str = env!("RECEIPTS_BUILD_COMMIT");
const DEPENDENCY_LOCK_DIGEST: &str = env!("RECEIPTS_LOCK_DIGEST");

fn main() {
    if let Err(error) = run() {
        eprintln!("receipts: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = match args.next() {
        Some(c) => c,
        None => {
            print_help();
            return Ok(());
        }
    };

    match command.as_str() {
        "--help" | "-h" | "help" => {
            print_help();
            Ok(())
        }
        "--version" | "-V" | "version" => {
            println!("receipts {VERSION}");
            Ok(())
        }
        "identity" => {
            println!(
                "{}",
                serde_json::json!({
                    "protocol_version": ENGINE_PROTOCOL_VERSION,
                    "engine_version": VERSION,
                    "build_commit": BUILD_COMMIT,
                    "dependency_lock_digest": DEPENDENCY_LOCK_DIGEST,
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                })
            );
            Ok(())
        }
        "doctor" => {
            let rest: Vec<String> = args.collect();
            let repo_root = parse_flag_value(&rest, "--repo-root")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
            let report = receipts_core::compiler::doctor::run_doctor(&repo_root);
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.ok {
                Ok(())
            } else {
                Err("doctor found one or more fail-closed installation problems".into())
            }
        }
        "project-public" => {
            let rest: Vec<String> = args.collect();
            let run_dir = PathBuf::from(
                parse_flag_value(&rest, "--run-dir")
                    .ok_or("`project-public` requires --run-dir <dir>")?,
            );
            let out = PathBuf::from(
                parse_flag_value(&rest, "--out").ok_or("`project-public` requires --out <file>")?,
            );
            preflight_run_dir(&run_dir)?;
            receipts_core::compiler::privacy::write_public_projection(&run_dir, &out)?;
            println!("public projection written: {}", out.display());
            Ok(())
        }
        "synthesize" => {
            let rest: Vec<String> = args.collect();
            let run_dir = PathBuf::from(
                parse_flag_value(&rest, "--run-dir")
                    .ok_or("`synthesize` requires --run-dir <dir>")?,
            );
            let summary = parse_flag_value(&rest, "--summary")
                .ok_or("`synthesize` requires --summary <text>")?;
            preflight_run_dir(&run_dir)?;
            record_synthesis(&run_dir, &summary)?;
            let report = compile_run_dir(&run_dir)?;
            println!(
                "synthesis recorded; packet={}",
                report.packet_path.display()
            );
            Ok(())
        }
        "gate" => {
            let run_dir = parse_run_dir(args.collect())?;
            preflight_run_dir(&run_dir)?;
            let report = receipts_core::compiler::gate::run_gate(&run_dir)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            if report.ok {
                Ok(())
            } else {
                Err("strict gate failed".into())
            }
        }
        "init" => {
            let rest: Vec<String> = args.collect();
            let dir = parse_path_arg(rest.clone(), "init")?;
            let repo_root = parse_flag_value(&rest, "--repo-root")
                .map(PathBuf::from)
                .unwrap_or(std::env::current_dir()?);
            init_run_dir(&dir, &repo_root)
        }
        "run" => {
            let rest: Vec<String> = args.collect();
            run_with_receipt(rest)
        }
        "check" => {
            let rest: Vec<String> = args.collect();
            check_with_binding(rest)
        }
        "ingest" => {
            let rest: Vec<String> = args.collect();
            let run_dir = PathBuf::from(
                parse_flag_value(&rest, "--run-dir").ok_or("`ingest` requires --run-dir <dir>")?,
            );
            let lane =
                parse_flag_value(&rest, "--lane").ok_or("`ingest` requires --lane <lane>")?;
            let agent_id =
                parse_flag_value(&rest, "--agent-id").ok_or("`ingest` requires --agent-id <id>")?;
            let from = PathBuf::from(
                parse_flag_value(&rest, "--from").ok_or("`ingest` requires --from <file>")?,
            );
            preflight_run_dir(&run_dir)?;
            let report = receipts_core::compiler::ingest::ingest_subagent(
                &run_dir,
                &lane,
                &agent_id,
                &from,
                &iso_now(),
                &chrono_like_stamp(),
            )?;
            println!("{}", serde_json::to_string(&report)?);
            Ok(())
        }
        "absorb" => absorb_lane(args.collect()),
        "conclude" => conclude_pass(args.collect()),
        "ready" => run_readiness(),
        "diff" => {
            let rest: Vec<String> = args.collect();
            diff_with_receipt(rest)
        }
        "resolve" => {
            let rest: Vec<String> = args.collect();
            resolve_worklist_item(rest)
        }
        "compile" => {
            let run_dir = parse_run_dir(args.collect())?;
            preflight_run_dir(&run_dir)?;
            let report = compile_run_dir(&run_dir)?;
            println!(
                "compiled run_dir={} snapshot={} packet={} decisions={} evidence={} verifier_findings={}",
                run_dir.display(),
                report.snapshot_path.display(),
                report.packet_path.display(),
                report.decision_log_path.display(),
                report.evidence_count,
                report.verifier_finding_count
            );
            Ok(())
        }
        "report" => {
            let run_dir = parse_run_dir(args.collect())?;
            preflight_run_dir(&run_dir)?;
            let report_path = generate_report(&run_dir)?;
            println!("report written: {}", report_path.display());
            Ok(())
        }
        "next" => {
            let rest: Vec<String> = args.collect();
            let as_json = rest.iter().any(|arg| arg == "--json");
            let run_dir = parse_run_dir(rest)?;
            preflight_run_dir(&run_dir)?;
            print!(
                "{}",
                receipts_core::compiler::brief::generate_brief(&run_dir, as_json)?
            );
            Ok(())
        }
        other => Err(format!("unknown command `{other}` — try `receipts --help`").into()),
    }
}

fn print_help() {
    println!(
        "receipts {VERSION} — deterministic packet compiler for AI agent runs

USAGE:
    receipts <COMMAND> [ARGS]

COMMANDS:
    identity                        Print the machine-readable engine identity
    doctor [--repo-root <path>]     Verify engine, executor key, schemas, check manifest,
                                    and model/runtime metadata availability
    project-public --run-dir <dir> --out <file>
                                    Build a deterministic allowlist-only public projection
    synthesize --run-dir <dir> --summary <text>
                                    Record the main-agent synthesis and advance the pass
    gate --run-dir <dir>            Recompile and apply the categorical strict gate
    init <dir> [--repo-root <path>]   Scaffold a run directory (repo_root defaults to cwd)
    run --run-dir <dir> [--lane L] [--agent-id A] [--label test:name] -- <command...>
    run --run-dir <dir> [--lane L] [--agent-id A] [--label test:name] --exe <program> [--arg <token>]...
                            Execute a command and mint a tamper-evident execution
                            receipt in receipts/receipts.jsonl (exit code = child's)
    check --run-dir <dir> --id <check-id>
                            Execute an engine-declared .receipts/checks.toml check
                            and bind it to exact subject, lock, environment, and claims
    ingest --run-dir <dir> --lane <lane> --agent-id <id> --from <file>
                            Quarantine and normalize a natural-prose or JSONL lane result
    absorb --run-dir <dir> --lane <lane> --agent-id <id> --from <file> [--no-diff]
                            Ingest, mint a work receipt, and recompile in one motion
    conclude --run-dir <dir> --synthesis <text> [--skip-report]
                            Synthesize, gate, report, and print the next brief
    ready                   Exercise the installed engine end to end
    diff --run-dir <dir> [--note <text>] [--patch]
                            Mint a WORK receipt: what changed in repo_root's tree
                            (numstat summary by default; --patch embeds the full
                            patch, hard-capped at 512KB). Work receipts attest
                            tree state and are invisible to claim attestation.
    resolve --run-dir <dir> --target <id> --reason <text> [--cite <source-id>]
                            Record a hash-chained adjudication clearing a blocking
                            worklist item (recompile to apply)
    compile --run-dir <dir> Compile a run directory into state/next_pass_packet.json
    report --run-dir <dir>  Render a human-readable state/report.html for the run
    next --run-dir <dir> [--json]
                            Print the compressed Prime brief: worklist first,
                            refutations, facts, lane digests with drill-down
                            handles, receipts, drift
    --version, -V           Print version
    --help, -h              Print this help

A run directory contains:
    manifest.json                   run identity (id, objective, created_at)
    task.md                         human-readable objective
    raw/                            quarantined raw subagent artifacts
    worker-results/evidence.jsonl   fenced evidence records (one JSON per line)
    verifier-results/findings.jsonl fenced verifier records (one JSON per line)

After compile, state/ holds next_pass_packet.json, snapshot.json, decision_log.jsonl.

See https://github.com/inchwormz/agent-receipts for the JS runtime (ingest, gate, readiness)."
    );
}

fn parse_run_dir(args: Vec<String>) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--run-dir" {
            return iter
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "`--run-dir` requires a path".into());
        }
    }
    Err("missing required `--run-dir <path>` — run `receipts --help` for usage".into())
}

fn parse_path_arg(args: Vec<String>, cmd: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg.starts_with("--") {
            skip_next = true; // flags in init take a value
            continue;
        }
        return Ok(PathBuf::from(arg));
    }
    Err(format!("`{cmd}` requires a directory path — try `receipts {cmd} my-run`").into())
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == flag)?;
    args.get(index + 1).cloned()
}

fn run_self(args: &[String]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    Ok(std::process::Command::new(std::env::current_exe()?)
        .args(args)
        .output()?)
}

fn absorb_lane(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let run_dir =
        parse_flag_value(&args, "--run-dir").ok_or("`absorb` requires --run-dir <dir>")?;
    let lane = parse_flag_value(&args, "--lane").ok_or("`absorb` requires --lane <lane>")?;
    let agent_id =
        parse_flag_value(&args, "--agent-id").ok_or("`absorb` requires --agent-id <id>")?;
    let from = parse_flag_value(&args, "--from").ok_or("`absorb` requires --from <file>")?;
    let ingest = run_self(&[
        "ingest".to_string(),
        "--run-dir".to_string(),
        run_dir.clone(),
        "--lane".to_string(),
        lane.clone(),
        "--agent-id".to_string(),
        agent_id,
        "--from".to_string(),
        from,
    ])?;
    if !ingest.status.success() {
        return Err(format!(
            "absorb ingest failed: {}",
            String::from_utf8_lossy(&ingest.stderr).trim()
        )
        .into());
    }
    let ingest_report: serde_json::Value = serde_json::from_slice(&ingest.stdout)?;
    let mut work_receipt = serde_json::Value::Null;
    if !args.iter().any(|arg| arg == "--no-diff") {
        let diff = run_self(&[
            "diff".to_string(),
            "--run-dir".to_string(),
            run_dir.clone(),
            "--note".to_string(),
            format!("post-lane {lane}"),
        ])?;
        if diff.status.success() {
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&diff.stdout) {
                work_receipt = value
                    .get("receipt")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
            }
        } else {
            eprintln!(
                "receipts absorb: warning: diff failed: {}",
                String::from_utf8_lossy(&diff.stderr).trim()
            );
        }
    }
    let compile = run_self(&["compile".to_string(), "--run-dir".to_string(), run_dir])?;
    if !compile.status.success() {
        return Err(format!(
            "absorb compile failed: {}",
            String::from_utf8_lossy(&compile.stderr).trim()
        )
        .into());
    }
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "lane": lane,
            "ingest": ingest_report,
            "work_receipt": work_receipt,
            "compiled": true,
        })
    );
    Ok(())
}

fn conclude_pass(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let run_dir =
        parse_flag_value(&args, "--run-dir").ok_or("`conclude` requires --run-dir <dir>")?;
    let synthesis =
        parse_flag_value(&args, "--synthesis").ok_or("`conclude` requires --synthesis <text>")?;
    let synthesize = run_self(&[
        "synthesize".to_string(),
        "--run-dir".to_string(),
        run_dir.clone(),
        "--summary".to_string(),
        synthesis,
    ])?;
    if !synthesize.status.success() {
        return Err(format!(
            "conclude synthesis failed: {}",
            String::from_utf8_lossy(&synthesize.stderr).trim()
        )
        .into());
    }
    let gate = run_self(&["gate".to_string(), "--run-dir".to_string(), run_dir.clone()])?;
    if !args.iter().any(|arg| arg == "--skip-report") {
        let report = run_self(&[
            "report".to_string(),
            "--run-dir".to_string(),
            run_dir.clone(),
        ])?;
        if !report.status.success() {
            eprintln!(
                "receipts conclude: warning: report failed: {}",
                String::from_utf8_lossy(&report.stderr).trim()
            );
        }
    }
    let next = run_self(&["next".to_string(), "--run-dir".to_string(), run_dir])?;
    if next.status.success() {
        print!("{}", String::from_utf8_lossy(&next.stdout));
    } else {
        eprintln!(
            "receipts conclude: warning: next failed: {}",
            String::from_utf8_lossy(&next.stderr).trim()
        );
    }
    if !gate.status.success() {
        std::process::exit(gate.status.code().unwrap_or(1));
    }
    Ok(())
}

struct ReadinessFixture(PathBuf);

impl Drop for ReadinessFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_readiness() -> Result<(), Box<dyn std::error::Error>> {
    let stamp = chrono_like_stamp();
    let root = std::env::current_dir()?;
    let fixture = ReadinessFixture(std::env::temp_dir().join(format!(
        "agent-receipts-readiness-{}-{stamp}",
        std::process::id()
    )));
    let run_dir = fixture.0.join("run");
    let init = run_self(&[
        "init".to_string(),
        run_dir.display().to_string(),
        "--repo-root".to_string(),
        root.display().to_string(),
    ])?;
    if !init.status.success() {
        return Err(format!(
            "readiness init failed: {}",
            String::from_utf8_lossy(&init.stderr).trim()
        )
        .into());
    }
    let lane = fixture.0.join("lane.md");
    fs::write(
        &lane,
        "```receipts-evidence-jsonl\n{\"id\":\"ev-readiness\",\"kind\":\"observation\",\"summary\":\"README is present\",\"source_ids\":[\"file:README.md:1\"]}\n```\n",
    )?;
    let ingest = run_self(&[
        "ingest".to_string(),
        "--run-dir".to_string(),
        run_dir.display().to_string(),
        "--lane".to_string(),
        "readiness".to_string(),
        "--agent-id".to_string(),
        "readiness-engine".to_string(),
        "--from".to_string(),
        lane.display().to_string(),
    ])?;
    if !ingest.status.success() {
        return Err(format!(
            "readiness ingest failed: {}",
            String::from_utf8_lossy(&ingest.stderr).trim()
        )
        .into());
    }
    let current = std::env::current_exe()?;
    let execution = run_self(&[
        "run".to_string(),
        "--run-dir".to_string(),
        run_dir.display().to_string(),
        "--label".to_string(),
        "test:readiness-identity".to_string(),
        "--exe".to_string(),
        current.display().to_string(),
        "--arg".to_string(),
        "identity".to_string(),
    ])?;
    if !execution.status.success() {
        return Err(format!(
            "readiness signed execution failed: {}",
            String::from_utf8_lossy(&execution.stderr).trim()
        )
        .into());
    }
    let synthesis = run_self(&[
        "synthesize".to_string(),
        "--run-dir".to_string(),
        run_dir.display().to_string(),
        "--summary".to_string(),
        "Readiness consumed the source-backed fixture.".to_string(),
    ])?;
    if !synthesis.status.success() {
        return Err(format!(
            "readiness synthesis failed: {}",
            String::from_utf8_lossy(&synthesis.stderr).trim()
        )
        .into());
    }
    let gate = std::process::Command::new(std::env::current_exe()?)
        .args(["gate", "--run-dir", run_dir.to_string_lossy().as_ref()])
        .env("RECEIPTS_MIN_AGENT_COVERAGE", "1")
        .output()?;
    if !gate.status.success() {
        return Err(format!(
            "readiness gate failed: stdout={} stderr={}",
            String::from_utf8_lossy(&gate.stdout).trim(),
            String::from_utf8_lossy(&gate.stderr).trim()
        )
        .into());
    }
    println!("receipts readiness: passed");
    Ok(())
}

fn charset_ok(value: &str) -> bool {
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, ':' | '.' | '_' | '/' | '-'))
}

/// Phase 2: Prime's typed adjudication. Appends a hash-chained resolution to
/// decisions/resolutions.jsonl; compile marks the matching worklist item
/// resolved on the next pass.
fn resolve_worklist_item(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let run_dir = PathBuf::from(
        parse_flag_value(&args, "--run-dir").ok_or("`resolve` requires --run-dir <dir>")?,
    );
    preflight_run_dir(&run_dir)?;
    let target = parse_flag_value(&args, "--target").ok_or(
        "`resolve` requires --target <id> (a contradiction, blocker-evidence, or finding id)",
    )?;
    let reason = parse_flag_value(&args, "--reason")
        .ok_or("`resolve` requires --reason \"<why this is adjudicated>\"")?;
    let cite = parse_flag_value(&args, "--cite");
    if !charset_ok(&target) {
        return Err(
            format!("--target `{target}` contains characters outside [A-Za-z0-9:._/-]").into(),
        );
    }
    if let Some(ref value) = cite {
        if !charset_ok(value) {
            return Err(
                format!("--cite `{value}` contains characters outside [A-Za-z0-9:._/-]").into(),
            );
        }
    }

    let record = receipts_core::compiler::resolutions::append_resolution(
        &run_dir,
        receipts_core::compiler::resolutions::ResolutionRecord {
            id: String::new(),
            target_id: target,
            reason,
            cite,
            resolved_at: iso_now(),
            writer: format!("receipts/{VERSION}"),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        },
    )?;
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "resolution": record.id,
            "target": record.target_id,
            "record_hash": record.record_hash,
            "next": "recompile the run (receipts compile --run-dir <dir>) to apply",
        })
    );
    Ok(())
}

/// M1: execute a command and mint an execution receipt the agent cannot
/// author. Exits with the CHILD's exit code so orchestrator scripting sees
/// reality; the receipt is minted either way.
fn run_with_receipt(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let (flags, command_line) = parse_run_invocation(args)?;
    let run_dir = PathBuf::from(
        parse_flag_value(&flags, "--run-dir").ok_or("`run` requires --run-dir <dir>")?,
    );
    preflight_run_dir(&run_dir)?;
    let lane = parse_flag_value(&flags, "--lane");
    let agent_id = parse_flag_value(&flags, "--agent-id");
    let label = parse_flag_value(&flags, "--label");
    if let Some(ref value) = label {
        // Labels are rendered inside briefs/suggested commands downstream;
        // enforce a shell-safe charset at the mint.
        if !charset_ok(value) {
            return Err(
                format!("--label `{value}` contains characters outside [A-Za-z0-9:._/-]").into(),
            );
        }
        if value == WORK_LABEL {
            return Err(format!(
                "--label `{WORK_LABEL}` is reserved for `receipts diff` work receipts"
            )
            .into());
        }
    }

    let repo_root: Option<String> = fs::read_to_string(run_dir.join("manifest.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value["repo_root"].as_str().map(str::to_string));

    let cwd = std::env::current_dir()?;
    let tree_before = git_tree_state(repo_root.as_deref());
    let started_at = iso_now();
    let start = std::time::Instant::now();
    let output = std::process::Command::new(&command_line[0])
        .args(&command_line[1..])
        .current_dir(&cwd)
        .output()
        .map_err(|err| {
            format!(
                "failed to launch `{}`: {err}. Note: shell builtins and .cmd scripts need an explicit shell, e.g. receipts run ... -- bash -lc \"<line>\"",
                command_line[0]
            )
        })?;
    let duration_ms = start.elapsed().as_millis() as u64;
    let ended_at = iso_now();
    let tree_after = git_tree_state(repo_root.as_deref());
    let exit_code = i64::from(output.status.code().unwrap_or(-1));

    let (stdout_hash, stdout_artifact) = store_artifact(&run_dir, &output.stdout)?;
    let (stderr_hash, stderr_artifact) = store_artifact(&run_dir, &output.stderr)?;
    let tail = |bytes: &[u8]| -> String {
        let text = String::from_utf8_lossy(bytes);
        let chars: Vec<char> = text.chars().collect();
        let start = chars.len().saturating_sub(2000);
        chars[start..].iter().collect()
    };

    let record = append_receipt(
        &run_dir,
        ReceiptRecord {
            id: String::new(),
            label,
            cmd: command_line,
            cwd: cwd.to_string_lossy().to_string(),
            exit_code,
            duration_ms,
            started_at,
            ended_at,
            stdout_hash,
            stderr_hash,
            stdout_tail: tail(&output.stdout),
            stderr_tail: tail(&output.stderr),
            tree_before,
            tree_after,
            lane,
            agent_id,
            writer: format!("receipts/{VERSION}"),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        },
    )?;

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "receipt": record.id,
            "record_hash": record.record_hash,
            "label": record.label,
            "exit_code": exit_code,
            "duration_ms": duration_ms,
            "stdout_artifact": stdout_artifact,
            "stderr_artifact": stderr_artifact,
            "cite_as": format!("receipt:{}", record.id),
        })
    );
    std::process::exit(exit_code as i32);
}

fn check_with_binding(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let run_dir = PathBuf::from(
        parse_flag_value(&args, "--run-dir").ok_or("`check` requires --run-dir <dir>")?,
    );
    let check_id = parse_flag_value(&args, "--id").ok_or("`check` requires --id <check-id>")?;
    preflight_run_dir(&run_dir)?;
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("manifest.json"))?)?;
    let repo_root = PathBuf::from(
        manifest["repo_root"]
            .as_str()
            .ok_or("`check` requires repo_root in manifest.json")?,
    );
    let checks = receipts_core::compiler::checks::load_manifest(&repo_root)?
        .ok_or("repo_root has no .receipts/checks.toml")?;
    let check = receipts_core::compiler::checks::find_check(&checks, &check_id)?;
    let attempt = receipts_core::compiler::checks::run_check(
        &run_dir,
        &repo_root,
        check,
        &format!("receipts/{VERSION}"),
    )?;
    println!("{}", serde_json::to_string(&attempt)?);
    if attempt.outcome != "passed" {
        std::process::exit(1);
    }
    Ok(())
}

fn parse_run_invocation(
    args: Vec<String>,
) -> Result<(Vec<String>, Vec<String>), Box<dyn std::error::Error>> {
    let mut scan = 0;
    let mut separator = None;
    while scan < args.len() {
        if args[scan] == "--" {
            separator = Some(scan);
            break;
        }
        if matches!(
            args[scan].as_str(),
            "--run-dir" | "--lane" | "--agent-id" | "--label" | "--exe" | "--arg"
        ) {
            scan += 2;
        } else {
            scan += 1;
        }
    }
    if let Some(separator) = separator {
        let flags = args[..separator].to_vec();
        if flags.iter().any(|arg| arg == "--exe" || arg == "--arg") {
            return Err("`run` cannot combine `--` with `--exe`/`--arg`".into());
        }
        validate_run_flags(&flags)?;
        let command = args[separator + 1..].to_vec();
        if command.is_empty() {
            return Err("`run` requires a command after `--`".into());
        }
        return Ok((flags, command));
    }

    let mut flags = Vec::new();
    let mut command = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--run-dir" | "--lane" | "--agent-id" | "--label" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("`{}` requires a value", args[index]))?;
                if value.starts_with("--") {
                    return Err(format!("`{}` requires a value", args[index]).into());
                }
                flags.extend([args[index].clone(), value.clone()]);
                index += 2;
            }
            "--exe" => {
                if !command.is_empty() {
                    return Err("`run` accepts exactly one `--exe`".into());
                }
                let executable = args.get(index + 1).ok_or("`--exe` requires a program")?;
                if executable.starts_with("--") {
                    return Err("`--exe` requires a program".into());
                }
                command.push(executable.clone());
                index += 2;
            }
            "--arg" => {
                if command.is_empty() {
                    return Err("`--arg` requires a preceding `--exe <program>`".into());
                }
                let value = args.get(index + 1).ok_or("`--arg` requires one token")?;
                command.push(value.clone());
                index += 2;
            }
            unknown => return Err(format!("unknown `run` argument `{unknown}`").into()),
        }
    }
    if command.is_empty() {
        return Err("`run` requires either `-- <command...>` or `--exe <program>`".into());
    }
    validate_run_flags(&flags)?;
    Ok((flags, command))
}

fn validate_run_flags(flags: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mut index = 0;
    while index < flags.len() {
        match flags[index].as_str() {
            "--run-dir" | "--lane" | "--agent-id" | "--label" => {
                if flags
                    .get(index + 1)
                    .is_none_or(|value| value.starts_with("--"))
                {
                    return Err(format!("`{}` requires a value", flags[index]).into());
                }
                index += 2;
            }
            unknown => return Err(format!("unknown `run` flag `{unknown}`").into()),
        }
    }
    Ok(())
}

/// Phase 1: mint a WORK receipt capturing what changed in repo_root's tree.
/// Numstat-only by default (no file contents -> no secret capture, bounded
/// size); `--patch` embeds the full patch, hard-capped at 512KB. The label is
/// always the constant `work:tree` - never caller-chosen - and both compile
/// and gate exclude that label from claim attestation.
fn diff_with_receipt(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let run_dir = PathBuf::from(
        parse_flag_value(&args, "--run-dir").ok_or("`diff` requires --run-dir <dir>")?,
    );
    preflight_run_dir(&run_dir)?;
    let note = parse_flag_value(&args, "--note");
    let want_patch = args.iter().any(|arg| arg == "--patch");
    let lane = parse_flag_value(&args, "--lane");
    let agent_id = parse_flag_value(&args, "--agent-id");

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("manifest.json"))?)?;
    let repo_root = manifest["repo_root"]
        .as_str()
        .ok_or("`diff` requires repo_root in manifest.json (re-init the run with --repo-root)")?
        .to_string();

    // Pathspec-exclude the run-dir tree so the engine's own journal churn
    // never shows up as "work". `.receipts` (and legacy `.mythos`) cover default locations; the
    // actual run dir is excluded too when it lives under repo_root elsewhere.
    let mut excludes: Vec<String> = vec![
        ":(exclude,top).receipts".to_string(),
        ":(exclude,top).mythos".to_string(),
    ];
    if let (Ok(run_abs), Ok(root_abs)) =
        (run_dir.canonicalize(), Path::new(&repo_root).canonicalize())
    {
        if let Ok(rel) = run_abs.strip_prefix(&root_abs) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            if !rel.is_empty() && !rel.starts_with(".receipts") && !rel.starts_with(".mythos") {
                excludes.push(format!(":(exclude,top){rel}"));
            }
        }
    }

    // Exclude-only pathspecs: git treats "-- :(exclude)..." as "everything
    // minus the excludes". Do NOT add a positive anchor like ":(top)." - it
    // silently suppresses all matches in status/diff (found by dogfooding:
    // a work receipt reported 0 files while ten sat modified).
    let git = |extra: &[&str]| -> Result<std::process::Output, Box<dyn std::error::Error>> {
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(&repo_root);
        cmd.args(extra);
        cmd.arg("--");
        for exclude in &excludes {
            cmd.arg(exclude);
        }
        Ok(cmd.output()?)
    };

    let started_at = iso_now();
    let start = std::time::Instant::now();
    let status_out = git(&["status", "--porcelain", "--untracked-files=all"])?;
    let numstat_out = git(&["diff", "--numstat", "HEAD"])?;
    if !status_out.status.success() || !numstat_out.status.success() {
        return Err(format!(
            "git failed under {repo_root}: {}",
            String::from_utf8_lossy(&numstat_out.stderr)
        )
        .into());
    }

    // Parse numstat: "<added>\t<removed>\t<path>" ("-" for binary).
    #[derive(serde::Serialize)]
    struct FileDelta {
        path: String,
        added: Option<u64>,
        removed: Option<u64>,
        status: String,
    }
    let mut files: Vec<FileDelta> = Vec::new();
    for line in String::from_utf8_lossy(&numstat_out.stdout).lines() {
        let mut parts = line.splitn(3, '\t');
        let (Some(a), Some(r), Some(p)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        files.push(FileDelta {
            path: p.trim().to_string(),
            added: a.parse().ok(),
            removed: r.parse().ok(),
            status: "modified".to_string(),
        });
    }
    let mut untracked = 0u64;
    for line in String::from_utf8_lossy(&status_out.stdout).lines() {
        if let Some(path) = line.strip_prefix("?? ") {
            untracked += 1;
            files.push(FileDelta {
                path: path.trim().to_string(),
                added: None,
                removed: None,
                status: "untracked".to_string(),
            });
        }
    }
    let total_added: u64 = files.iter().filter_map(|f| f.added).sum();
    let total_removed: u64 = files.iter().filter_map(|f| f.removed).sum();
    let total_files = files.len();
    // Deterministic order: biggest deltas first, path as tiebreak; top 100
    // inline, remainder counted.
    files.sort_by(|left, right| {
        let l = left.added.unwrap_or(0) + left.removed.unwrap_or(0);
        let r = right.added.unwrap_or(0) + right.removed.unwrap_or(0);
        r.cmp(&l).then_with(|| left.path.cmp(&right.path))
    });
    let truncated = files.len() > 100;
    files.truncate(100);

    let mut artifact = serde_json::json!({
        "work": "tree",
        "note": note,
        "files": files,
        "total_files": total_files,
        "total_added": total_added,
        "total_removed": total_removed,
        "untracked": untracked,
        "truncated": truncated,
    });
    if want_patch {
        let patch_out = git(&["diff", "HEAD"])?;
        const PATCH_CAP: usize = 512 * 1024;
        if patch_out.stdout.len() > PATCH_CAP {
            return Err(format!(
                "--patch refused: patch is {} bytes (cap {PATCH_CAP}). Use the numstat summary, or narrow the tree.",
                patch_out.stdout.len()
            )
            .into());
        }
        artifact["patch"] =
            serde_json::Value::String(String::from_utf8_lossy(&patch_out.stdout).to_string());
    }
    let artifact_bytes = serde_json::to_vec_pretty(&artifact)?;
    let (artifact_hash, artifact_rel) = store_artifact(&run_dir, &artifact_bytes)?;
    let (stderr_hash, _) = store_artifact(&run_dir, &numstat_out.stderr)?;

    let duration_ms = start.elapsed().as_millis() as u64;
    let ended_at = iso_now();
    let tree = git_tree_state(Some(&repo_root));
    let summary_tail = format!(
        "{total_files} file(s) changed, +{total_added}/-{total_removed}, {untracked} untracked"
    );

    let record = append_receipt(
        &run_dir,
        ReceiptRecord {
            id: String::new(),
            label: Some(WORK_LABEL.to_string()),
            cmd: vec![
                "git".to_string(),
                "diff".to_string(),
                "--numstat".to_string(),
                "HEAD".to_string(),
            ],
            cwd: repo_root.clone(),
            exit_code: 0,
            duration_ms,
            started_at,
            ended_at,
            stdout_hash: artifact_hash,
            stderr_hash,
            stdout_tail: summary_tail.clone(),
            stderr_tail: String::new(),
            tree_before: tree.clone(),
            tree_after: tree,
            lane,
            agent_id,
            writer: format!("receipts/{VERSION}"),
            prev_record_hash: String::new(),
            record_hash: String::new(),
        },
    )?;

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "receipt": record.id,
            "record_hash": record.record_hash,
            "label": WORK_LABEL,
            "summary": summary_tail,
            "artifact": artifact_rel,
        })
    );
    Ok(())
}

fn preflight_run_dir(run_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if !run_dir.exists() {
        return Err(format!(
            "run directory `{}` does not exist — scaffold one with `receipts init {}`",
            run_dir.display(),
            run_dir.display()
        )
        .into());
    }
    let manifest = run_dir.join("manifest.json");
    if !manifest.exists() {
        return Err(format!(
            "`{}` is missing manifest.json — scaffold a valid run dir with `receipts init {}`",
            run_dir.display(),
            run_dir.display()
        )
        .into());
    }
    Ok(())
}

fn record_synthesis(run_dir: &Path, summary: &str) -> Result<(), Box<dyn std::error::Error>> {
    let summary = summary.trim();
    if summary.is_empty() {
        return Err("synthesis summary must not be empty".into());
    }
    // Validate and refresh the exact state being consumed before recording
    // that it was synthesized.
    compile_run_dir(run_dir)?;
    let stamp = chrono_like_stamp();
    let observed_at = iso_now();
    let raw_name = format!("codex-synthesis-{stamp}.md");
    let raw_source_id = format!("raw:{raw_name}");
    fs::write(
        run_dir.join("raw").join(&raw_name),
        format!("# Codex Synthesis {stamp}\n\n{summary}\n"),
    )?;

    let evidence_path = run_dir.join("worker-results/evidence.jsonl");
    let mut evidence_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(evidence_path)?;
    serde_json::to_writer(
        &mut evidence_file,
        &serde_json::json!({
            "id": format!("ev-codex-synthesis-{stamp}"),
            "kind": "codex-synthesis",
            "summary": summary,
            "source_ids": [raw_source_id.clone()],
            "observed_at": observed_at,
        }),
    )?;
    use std::io::Write as _;
    evidence_file.write_all(b"\n")?;
    evidence_file.flush()?;

    let findings_path = run_dir.join("verifier-results/findings.jsonl");
    let mut findings: Vec<serde_json::Value> = fs::read_to_string(&findings_path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let mut consumed_pending = false;
    for finding in &mut findings {
        if finding.get("id").and_then(serde_json::Value::as_str)
            != Some("vf-codex-synthesis-pending")
        {
            continue;
        }
        consumed_pending = true;
        let object = finding
            .as_object_mut()
            .ok_or("verifier finding must be an object")?;
        object.insert(
            "summary".to_string(),
            serde_json::Value::String(format!("Codex synthesis consumed packet state: {summary}")),
        );
        object.insert(
            "status".to_string(),
            serde_json::Value::String("passed".to_string()),
        );
        object.insert("verifier_score".to_string(), serde_json::json!(0.9));
        object.insert(
            "finding_kind".to_string(),
            serde_json::Value::String("synthesis".to_string()),
        );
        let mut source_ids: Vec<String> = object
            .get("source_ids")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_string)
            .collect();
        if !source_ids.contains(&raw_source_id) {
            source_ids.push(raw_source_id.clone());
        }
        object.insert("source_ids".to_string(), serde_json::json!(source_ids));
    }
    if !consumed_pending {
        findings.push(serde_json::json!({
            "id": format!("vf-codex-synthesis-{stamp}"),
            "summary": format!("Codex synthesis consumed packet state: {summary}"),
            "status": "passed",
            "verifier_score": 0.9,
            "source_ids": [raw_source_id],
            "finding_kind": "synthesis",
        }));
    }
    let mut findings_bytes = Vec::new();
    for finding in findings {
        serde_json::to_writer(&mut findings_bytes, &finding)?;
        findings_bytes.push(b'\n');
    }
    fs::write(findings_path, findings_bytes)?;

    let manifest_path = run_dir.join("manifest.json");
    let mut manifest: serde_json::Value = serde_json::from_slice(&fs::read(&manifest_path)?)?;
    let current = manifest
        .get("pass_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("pass-0001");
    let next = current
        .strip_prefix("pass-")
        .and_then(|number| number.parse::<u64>().ok())
        .map(|number| format!("pass-{:04}", number + 1))
        .unwrap_or_else(|| "pass-0002".to_string());
    manifest["pass_id"] = serde_json::Value::String(next);
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');
    fs::write(manifest_path, manifest_bytes)?;
    Ok(())
}

fn init_run_dir(dir: &Path, repo_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    if dir.exists() && fs::read_dir(dir)?.next().is_some() {
        return Err(format!(
            "`{}` exists and is not empty — refusing to overwrite. Pick a new path.",
            dir.display()
        )
        .into());
    }

    fs::create_dir_all(dir.join("raw"))?;
    fs::create_dir_all(dir.join("worker-results"))?;
    fs::create_dir_all(dir.join("verifier-results"))?;

    let run_id = format!("run-{}", chrono_like_stamp());
    let objective = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("new-run")
        .to_string();

    let manifest = format!(
        "{{\n  \"run_id\": \"{}\",\n  \"objective_id\": \"obj-{}\",\n  \"objective\": \"{}\",\n  \"branch_id\": \"main\",\n  \"pass_id\": \"pass-0001\",\n  \"created_at\": \"{}\",\n  \"repo_root\": {}\n}}\n",
        run_id,
        chrono_like_stamp(),
        objective.replace('"', "\\\""),
        iso_now(),
        json_escape_string(&repo_root.to_string_lossy())
    );
    fs::write(dir.join("manifest.json"), manifest)?;

    let task = format!(
        "# {}\n\nDescribe the objective of this run here.\n",
        objective
    );
    fs::write(dir.join("task.md"), task)?;

    let objective_md = format!(
        "# Objective\n\n{}\n\n# Note\n\nThis run was scaffolded by `receipts init`. Ingest subagent output with `receipts ingest` or append evidence directly to worker-results/evidence.jsonl.\n",
        objective
    );
    fs::write(dir.join("raw/objective.md"), objective_md)?;

    let now = iso_now();
    let seed_evidence = format!(
        "{{\"id\":\"ev-objective\",\"kind\":\"objective\",\"summary\":{},\"source_ids\":[\"raw:objective.md\"],\"observed_at\":\"{}\"}}\n",
        json_escape_string(&objective),
        now
    );
    fs::write(dir.join("worker-results/evidence.jsonl"), seed_evidence)?;

    // F5: this id MUST match what the synthesis recorder consumes
    // (`vf-codex-synthesis-pending`) — the Rust and JS halves shipping
    // different ids made every Rust-scaffolded run permanently gate-red.
    let seed_finding = "{\"id\":\"vf-codex-synthesis-pending\",\"summary\":\"Codex synthesis has not consumed this packet yet\",\"status\":\"pending\",\"verifier_score\":0.0,\"source_ids\":[\"raw:objective.md\"],\"finding_kind\":\"synthesis\"}\n".to_string();
    fs::write(dir.join("verifier-results/findings.jsonl"), seed_finding)?;

    println!(
        "scaffolded run directory: {}\n\
         next steps:\n\
           1. append evidence records to {}/worker-results/evidence.jsonl\n\
           2. append verifier records to {}/verifier-results/findings.jsonl\n\
           3. run `receipts compile --run-dir {}`\n\
         \n\
         for the full subagent ingest + strict gate flow, install the JS runtime:\n\
           git clone https://github.com/inchwormz/agent-receipts && cd agent-receipts && npm run ready",
        dir.display(),
        dir.display(),
        dir.display(),
        dir.display()
    );
    Ok(())
}

fn json_escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, min, sec) = unix_to_utc(secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    )
}

fn chrono_like_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, day, hour, min, sec) = unix_to_utc(secs);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        year, month, day, hour, min, sec
    )
}

// Minimal inline UTC conversion to avoid adding a `chrono` dependency.
fn unix_to_utc(mut secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let sec = (secs % 60) as u32;
    secs /= 60;
    let min = (secs % 60) as u32;
    secs /= 60;
    let hour = (secs % 24) as u32;
    let days = secs / 24;

    // Days since 1970-01-01 → civil date. Howard Hinnant's chrono algorithm.
    let z = days as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32, hour, min, sec)
}
