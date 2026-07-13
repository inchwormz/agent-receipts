#!/usr/bin/env node
// receipts CLI — Node wrapper around the Rust `receipts-core` binary plus the
// JS pipeline (ingest, strict-gate, readiness). This is what an npm-installed
// user actually interacts with.
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";
import fs from "node:fs";

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.dirname(here);

const COMMANDS = {
  init: { script: null, description: "Scaffold a minimal run directory (delegates to receipts-core binary)" },
  run: { script: null, description: "Execute a command and mint a tamper-evident execution receipt (delegates to receipts-core binary)" },
  diff: { script: null, description: "Mint a WORK receipt of what changed in repo_root's tree (delegates to receipts-core binary)" },
  resolve: { script: null, description: "Record a hash-chained adjudication clearing a blocking worklist item (delegates to receipts-core binary)" },
  next: { script: null, description: "Print the compressed Prime brief for a run (delegates to receipts-core binary)" },
  compile: { script: "driver.mjs", description: "Compile a run directory into state/next_pass_packet.json" },
  ingest: { script: "scripts/ingest-subagent.mjs", description: "Ingest a subagent markdown file into evidence/findings JSONL" },
  absorb: { script: null, description: "One motion per completed lane: ingest -> diff -> recompile" },
  conclude: { script: null, description: "One motion to end a pass: synthesis -> gate -> report -> next" },
  gate: { script: "scripts/strict-gate.mjs", description: "Verify a run dir passes the strict quality gate" },
  ready: { script: "scripts/readiness.mjs", description: "Run the end-to-end readiness fixture" },
  help: { script: null, description: "Print this help" },
  version: { script: null, description: "Print the receipts version" },
};

function printHelp() {
  const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
  process.stdout.write(
    `receipts ${pkg.version} — explicit-state recurrent synthesis for AI agent runs\n\n` +
      `USAGE:\n    receipts <COMMAND> [ARGS]\n\n` +
      `COMMANDS:\n` +
      Object.entries(COMMANDS)
        .map(([name, info]) => `    ${name.padEnd(10)}  ${info.description}`)
        .join("\n") +
      `\n\n` +
      `EXAMPLES:\n` +
      `    receipts ready                         # confirm the pipeline works end-to-end\n` +
      `    receipts init my-run                   # scaffold a fresh run directory\n` +
      `    receipts compile --run-dir my-run      # compile a run\n` +
      `    receipts ingest --run-dir my-run --lane L1 --agent-id a --from agent.md\n` +
      `    receipts absorb --run-dir my-run --lane L1 --agent-id a --from agent.md\n` +
      `    receipts conclude --run-dir my-run --synthesis "what happened this pass"\n` +
      `    receipts gate --run-dir my-run\n\n` +
      `The Rust compiler binary \`receipts-core\` is required for compile. Install it with\n` +
      `\`cargo install receipts\` if it is not on PATH.\n`
  );
}

function which(cmd) {
  const probe = spawnSync(process.platform === "win32" ? "where" : "which", [cmd], {
    stdio: "pipe",
    encoding: "utf8",
  });
  return probe.status === 0 ? probe.stdout.split(/\r?\n/)[0].trim() : null;
}

// ---------------------------------------------------------------------------
// Loop composites: `absorb` and `conclude`. Dispatcher-level orchestration
// only — each step below is a spawn of an existing command/script; no new
// engine semantics live here. See the binding spec at
// .mythos/runs/20260712T234143Z-collapse-the-prime-loop-ceremony-and-truth-the-d/briefs/composites-spec.md
// ---------------------------------------------------------------------------

// Mirrors the `value()` helper in scripts/ingest-subagent.mjs: a flag whose
// next token looks like another flag (starts with `--`) is treated as
// missing, unless the caller opts in to free-text values (e.g. --synthesis).
function flagValue(argv, flag, { allowDashPrefixed = false } = {}) {
  const index = argv.indexOf(flag);
  if (index === -1) return null;
  const next = argv[index + 1];
  if (next === undefined) return null;
  if (!allowDashPrefixed && next.startsWith("--")) return null;
  return next;
}

function hasFlag(argv, flag) {
  return argv.includes(flag);
}

// Spawn a package-local Node script by path, capturing stdout/stderr instead
// of inheriting them (the caller decides what to forward). No shell here,
// matching the existing script-dispatch below: process.execPath and the
// script path are both full, resolved paths, so Windows can exec them
// directly — routing them through cmd.exe (shell: true) instead breaks the
// moment either path contains a space (e.g. `C:\Program Files\nodejs\node.exe`),
// since cmd.exe then splits the command at that space.
function spawnScript(scriptRelPath, scriptArgs) {
  return spawnSync(process.execPath, [path.join(root, scriptRelPath), ...scriptArgs], {
    encoding: "utf8",
  });
}

// Spawn the resolved `receipts-core` binary (a full path from which()/where). Same
// no-shell reasoning as spawnScript.
function spawnCore(corePath, coreArgs) {
  return spawnSync(corePath, coreArgs, { encoding: "utf8" });
}

function firstStderrLine(result, fallback) {
  const line = (result.stderr || "").trim().split(/\r?\n/)[0];
  return line || fallback;
}

function cmdAbsorb(args) {
  const runDir = flagValue(args, "--run-dir");
  const lane = flagValue(args, "--lane");
  const agentId = flagValue(args, "--agent-id");
  const from = flagValue(args, "--from");
  const noDiff = hasFlag(args, "--no-diff");
  if (!runDir || !lane || !agentId || !from) {
    process.stderr.write(
      "Usage: receipts absorb --run-dir <d> --lane <l> --agent-id <a> --from <file> [--no-diff]\n",
    );
    return 2;
  }

  // Step 1: ingest-subagent.mjs. FATAL on failure — propagate exit code + stderr.
  const ingestResult = spawnScript("scripts/ingest-subagent.mjs", [
    "--run-dir",
    runDir,
    "--lane",
    lane,
    "--agent-id",
    agentId,
    "--from",
    from,
  ]);
  if (ingestResult.error) {
    process.stderr.write(`receipts absorb: failed to run ingest-subagent.mjs: ${ingestResult.error.message}\n`);
    return 1;
  }
  if (ingestResult.status !== 0) {
    if (ingestResult.stderr) process.stderr.write(ingestResult.stderr);
    return ingestResult.status ?? 1;
  }
  let ingestJson;
  try {
    ingestJson = JSON.parse(ingestResult.stdout);
  } catch (error) {
    process.stderr.write(
      `receipts absorb: ingest-subagent.mjs did not print valid JSON to stdout: ${error.message}\n`,
    );
    return 1;
  }

  // Step 2: `receipts-core diff` unless --no-diff. NON-FATAL — repo_root may not be
  // a git repo, or the binary may not be installed; warn and continue.
  let workReceipt = null;
  if (!noDiff) {
    const core = which("receipts-core");
    if (!core) {
      process.stderr.write("receipts absorb: warning: `receipts-core` binary not found on PATH; skipping diff receipt\n");
    } else {
      const diffResult = spawnCore(core, ["diff", "--run-dir", runDir, "--note", `post-lane ${lane}`]);
      if (diffResult.error) {
        process.stderr.write(`receipts absorb: warning: receipts-core diff failed to launch (${diffResult.error.message}); continuing without a work receipt\n`);
      } else if (diffResult.status !== 0) {
        process.stderr.write(
          `receipts absorb: warning: receipts-core diff exited ${diffResult.status} (${firstStderrLine(diffResult, "no stderr")}); continuing without a work receipt\n`,
        );
      } else {
        try {
          workReceipt = JSON.parse(diffResult.stdout).receipt ?? null;
        } catch (error) {
          process.stderr.write("receipts absorb: warning: receipts-core diff did not print valid JSON; continuing without a work receipt\n");
        }
      }
    }
  }

  // Step 3: recompile. FATAL on failure.
  const compileResult = spawnScript("driver.mjs", ["--run-dir", runDir]);
  if (compileResult.error) {
    process.stderr.write(`receipts absorb: failed to run driver.mjs: ${compileResult.error.message}\n`);
    return 1;
  }
  if (compileResult.status !== 0) {
    if (compileResult.stderr) process.stderr.write(compileResult.stderr);
    return compileResult.status ?? 1;
  }

  process.stdout.write(
    `${JSON.stringify({
      ok: true,
      lane,
      ingest: ingestJson,
      work_receipt: workReceipt,
      compiled: true,
    })}\n`,
  );
  return 0;
}

function cmdConclude(args) {
  const runDir = flagValue(args, "--run-dir");
  const synthesis = flagValue(args, "--synthesis", { allowDashPrefixed: true });
  const skipReport = hasFlag(args, "--skip-report");
  if (!runDir || !synthesis) {
    process.stderr.write('Usage: receipts conclude --run-dir <d> --synthesis "<text>" [--skip-report]\n');
    return 2;
  }

  // Step 0: recompile FIRST. In the natural loop Prime mints receipts
  // between the last absorb and conclude, so the packet is almost always
  // stale here - record-synthesis fails closed on staleness by design.
  // (Field find, 2026-07-13: the first conclude of the first field run hit
  // exactly this; the spec missed it.) FATAL on failure.
  const precompile = spawnScript("driver.mjs", ["--run-dir", runDir]);
  if (precompile.error || precompile.status !== 0) {
    if (precompile.stderr) process.stderr.write(precompile.stderr);
    process.stderr.write("receipts conclude: pre-synthesis recompile failed\n");
    return precompile.status ?? 1;
  }

  // Step 1: record synthesis + recompile. FATAL on failure. Suppress its
  // stdout (the packet dump); let stderr through.
  const synthesisResult = spawnScript("driver.mjs", ["--run-dir", runDir, "--record-synthesis", synthesis]);
  if (synthesisResult.error) {
    process.stderr.write(`receipts conclude: failed to run driver.mjs: ${synthesisResult.error.message}\n`);
    return 1;
  }
  if (synthesisResult.stderr) process.stderr.write(synthesisResult.stderr);
  if (synthesisResult.status !== 0) {
    return synthesisResult.status ?? 1;
  }

  // Step 2: strict gate. ALWAYS write its stdout to state/gate-report.json,
  // green or red; remember its exit code — that is conclude's exit code.
  const gateResult = spawnScript("scripts/strict-gate.mjs", ["--run-dir", runDir]);
  if (gateResult.error) {
    process.stderr.write(`receipts conclude: failed to run strict-gate.mjs: ${gateResult.error.message}\n`);
    return 1;
  }
  fs.mkdirSync(path.join(runDir, "state"), { recursive: true });
  fs.writeFileSync(path.join(runDir, "state", "gate-report.json"), gateResult.stdout ?? "", "utf8");
  const gateExitCode = gateResult.status ?? 1;

  // Step 3: human-readable report, unless --skip-report. Non-fatal.
  if (!skipReport) {
    const core = which("receipts-core");
    if (!core) {
      process.stderr.write("receipts conclude: warning: `receipts-core` binary not found on PATH; skipping report\n");
    } else {
      const reportResult = spawnCore(core, ["report", "--run-dir", runDir]);
      if (reportResult.error) {
        process.stderr.write(`receipts conclude: warning: receipts-core report failed to launch (${reportResult.error.message})\n`);
      } else if (reportResult.status !== 0) {
        process.stderr.write(
          `receipts conclude: warning: receipts-core report exited ${reportResult.status} (${firstStderrLine(reportResult, "no stderr")})\n`,
        );
      }
    }
  }

  // Step 4: the compressed Prime brief — this is what Prime reads next.
  const coreNext = which("receipts-core");
  if (!coreNext) {
    process.stderr.write(
      "receipts: the `receipts-core` binary is not on PATH.\nInstall it with: cargo install receipts\n",
    );
    return gateExitCode;
  }
  const nextResult = spawnCore(coreNext, ["next", "--run-dir", runDir]);
  if (nextResult.error) {
    process.stderr.write(`receipts conclude: warning: receipts-core next failed to launch (${nextResult.error.message})\n`);
  } else {
    if (nextResult.stdout) process.stdout.write(nextResult.stdout);
    if (nextResult.stderr) process.stderr.write(nextResult.stderr);
  }

  // Fail-closed: a red run concludes red, regardless of report/next outcome.
  return gateExitCode;
}

function run(command, args) {
  switch (command) {
    case "help":
    case "--help":
    case "-h":
      printHelp();
      return 0;
    case "version":
    case "--version":
    case "-V": {
      const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
      process.stdout.write(`receipts ${pkg.version}\n`);
      return 0;
    }
    case "absorb":
      return cmdAbsorb(args);
    case "conclude":
      return cmdConclude(args);
    case "init":
    case "run":
    case "diff":
    case "resolve":
    case "next": {
      const core = which("receipts-core");
      if (!core) {
        process.stderr.write(
          "receipts: the `receipts-core` binary is not on PATH.\n" +
            "Install it with: cargo install receipts\n"
        );
        return 127;
      }
      const r = spawnSync(core, [command, ...args], { stdio: "inherit" });
      return r.status ?? 1;
    }
  }

  const info = COMMANDS[command];
  if (!info || !info.script) {
    process.stderr.write(`receipts: unknown command \`${command}\` — try \`receipts help\`\n`);
    return 2;
  }
  const scriptPath = path.join(root, info.script);
  if (!fs.existsSync(scriptPath)) {
    process.stderr.write(`receipts: missing ${info.script} — is this package installed correctly?\n`);
    return 1;
  }
  const r = spawnSync(process.execPath, [scriptPath, ...args], { stdio: "inherit" });
  return r.status ?? 1;
}

const [, , command = "help", ...rest] = process.argv;
process.exit(run(command, rest));
