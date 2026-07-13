#!/usr/bin/env node
// Receipts Codex driver.
//
// Codex is the vessel/main brain. This local driver only creates explicit run
// state, invokes receipts-compiler, and prints the packet Codex should consume.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const compilerDir = path.join(here, "receipts-compiler");

function usage() {
  return [
    "Usage:",
    '  node driver.mjs "<objective>"',
    "  node driver.mjs --run-dir <path>",
    '  node driver.mjs --run-dir <path> --record-synthesis "<summary>"',
    "",
    "Creates or compiles a Receipts run directory, then prints the source-backed",
    "next-pass packet for Codex to synthesize from.",
  ].join("\n");
}

function fail(message, code = 1) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function mkdirp(dir) {
  fs.mkdirSync(dir, { recursive: true });
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function appendJsonl(file, value) {
  fs.appendFileSync(file, `${JSON.stringify(value)}\n`, "utf8");
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs
    .readFileSync(file, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function writeJsonl(file, values) {
  fs.writeFileSync(file, `${values.map((value) => JSON.stringify(value)).join("\n")}\n`, "utf8");
}

function walkFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const files = [];
  const stack = [dir];
  while (stack.length) {
    const current = stack.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else {
        files.push(full);
      }
    }
  }
  return files;
}

function mtimeMs(file) {
  return fs.existsSync(file) ? fs.statSync(file).mtimeMs : 0;
}

function inputFiles(runDir) {
  const inputDirs = ["raw", "worker-results", "verifier-results", "receipts", "decisions"];
  return [
    path.join(runDir, "manifest.json"),
    path.join(runDir, "task.md"),
    ...inputDirs.flatMap((dir) => walkFiles(path.join(runDir, dir))),
  ].filter((file) => fs.existsSync(file)).sort();
}

function fnv1aHash(buffer) {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const byte of buffer) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

function inputFingerprint(runDir) {
  return inputFiles(runDir).map((file) => {
    const stat = fs.statSync(file);
    return {
      path: path.relative(runDir, file).replace(/\\/g, "/"),
      size: stat.size,
      hash: fnv1aHash(fs.readFileSync(file)),
    };
  });
}

function fingerprintPath(runDir) {
  return path.join(runDir, "state", "input_fingerprint.json");
}

function readStoredInputFingerprint(runDir) {
  const file = fingerprintPath(runDir);
  return fs.existsSync(file) ? readJson(file) : null;
}

function assertFreshPacketForSynthesis(runDir) {
  const packetPath = path.join(runDir, "state", "next_pass_packet.json");
  if (!fs.existsSync(packetPath)) {
    fail("missing next_pass_packet.json; run driver.mjs --run-dir before --record-synthesis", 1);
  }
  const stored = readStoredInputFingerprint(runDir);
  if (!stored || JSON.stringify(stored) !== JSON.stringify(inputFingerprint(runDir))) {
    fail("next_pass_packet.json is stale; run driver.mjs --run-dir before --record-synthesis", 1);
  }
}

function utcStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
}

function slugify(input) {
  return input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48) || "objective";
}

function parseArgs(argv) {
  if (argv.length === 0) {
    fail(usage(), 2);
  }
  if (argv.length === 1 && ["--help", "-h", "help"].includes(argv[0])) {
    process.stdout.write(`${usage()}\n`);
    process.exit(0);
  }

  const runDirFlag = argv.indexOf("--run-dir");
  const recordSynthesisFlag = argv.indexOf("--record-synthesis");
  if (recordSynthesisFlag !== -1) {
    const runDir = argv[runDirFlag + 1];
    const summary = argv[recordSynthesisFlag + 1];
    if (runDirFlag === -1 || !runDir || runDir.startsWith("--")) {
      fail("Missing value for --run-dir", 2);
    }
    if (!summary || summary.startsWith("--")) {
      fail("Missing summary for --record-synthesis", 2);
    }
    return {
      mode: "record-synthesis",
      runDir: path.resolve(runDir),
      summary,
    };
  }

  if (runDirFlag !== -1) {
    const runDir = argv[runDirFlag + 1];
    if (!runDir) fail("Missing value for --run-dir", 2);
    return { mode: "existing-run-dir", runDir: path.resolve(runDir) };
  }

  const objective = argv.join(" ").trim();
  if (!objective) fail(usage(), 2);
  // Flag-like objectives are always a mistake - the field produced a run
  // directory literally named "help" from a stray `--help` (2026-07-13).
  if (objective.startsWith("-")) {
    fail(`unrecognized flag \`${argv[0]}\` - not creating a run named after it\n\n${usage()}`, 2);
  }
  return { mode: "new-objective", objective };
}

function nextPassId(passId) {
  const match = /^pass-(\d+)$/.exec(String(passId || ""));
  if (!match) return "pass-0002";
  const width = match[1].length;
  return `pass-${String(Number(match[1]) + 1).padStart(width, "0")}`;
}

function recordSynthesis(runDir, summary) {
  assertFreshPacketForSynthesis(runDir);
  const manifestPath = path.join(runDir, "manifest.json");
  const manifest = readJson(manifestPath);
  const stamp = utcStamp();
  const observedAt = new Date().toISOString();
  const rawName = `codex-synthesis-${stamp}.md`;
  const rawSourceId = `raw:${rawName}`;
  const rawPath = path.join(runDir, "raw", rawName);
  const evidencePath = path.join(runDir, "worker-results", "evidence.jsonl");
  const verifierPath = path.join(runDir, "verifier-results", "findings.jsonl");

  mkdirp(path.dirname(rawPath));
  mkdirp(path.dirname(evidencePath));
  mkdirp(path.dirname(verifierPath));

  fs.writeFileSync(
    rawPath,
    [`# Codex Synthesis ${stamp}`, "", summary.trim(), ""].join("\n"),
    "utf8",
  );

  appendJsonl(evidencePath, {
    id: `ev-codex-synthesis-${stamp}`,
    kind: "codex-synthesis",
    summary: summary.trim(),
    source_ids: [rawSourceId],
    observed_at: observedAt,
  });

  let consumedPending = false;
  const findings = readJsonl(verifierPath).map((finding) => {
    if (finding.id !== "vf-codex-synthesis-pending") return finding;
    consumedPending = true;
    return {
      ...finding,
      summary: `Codex synthesis consumed packet state: ${summary.trim()}`,
      status: "passed",
      verifier_score: 0.9,
      source_ids: [...new Set([...(finding.source_ids || []), rawSourceId])],
      // F6: infrastructure findings carry a typed role; the strict gate keys
      // its exemptions on this, never on free text.
      finding_kind: "synthesis",
    };
  });

  if (!consumedPending) {
    findings.push({
      id: `vf-codex-synthesis-${stamp}`,
      summary: `Codex synthesis consumed packet state: ${summary.trim()}`,
      status: "passed",
      verifier_score: 0.9,
      source_ids: [rawSourceId],
      finding_kind: "synthesis",
    });
  }

  writeJsonl(verifierPath, findings);
  writeJson(manifestPath, {
    ...manifest,
    pass_id: nextPassId(manifest.pass_id),
  });
}

function createRunDir(objective) {
  const stamp = utcStamp();
  const runId = `run-${stamp}`;
  // F10: runs live under the PROJECT being worked on (cwd), never inside the
  // installed package directory.
  const runDir = path.join(process.cwd(), ".receipts", "runs", `${stamp}-${slugify(objective)}`);

  mkdirp(path.join(runDir, "raw"));
  mkdirp(path.join(runDir, "worker-results"));
  mkdirp(path.join(runDir, "verifier-results"));

  writeJson(path.join(runDir, "manifest.json"), {
    objective_id: `obj-${stamp}`,
    run_id: runId,
    branch_id: "main",
    pass_id: "pass-0001",
    created_at: new Date().toISOString(),
    // F3: file citations resolve against the project the run was created
    // from — recorded here, never inferred from any package location.
    repo_root: process.cwd(),
  });

  fs.writeFileSync(path.join(runDir, "task.md"), `${objective}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "raw", "objective.md"),
    [
      "# Objective",
      "",
      objective,
      "",
      "# Codex Vessel Note",
      "",
      "Codex is the main brain. The local driver only compiles explicit state.",
      "",
    ].join("\n"),
    "utf8",
  );

  appendJsonl(path.join(runDir, "worker-results", "evidence.jsonl"), {
    id: "ev-objective",
    kind: "objective",
    summary: objective,
    source_ids: ["raw:objective.md"],
    observed_at: new Date().toISOString(),
  });

  appendJsonl(path.join(runDir, "verifier-results", "findings.jsonl"), {
    id: "vf-codex-synthesis-pending",
    summary: "Codex synthesis has not consumed this packet yet",
    status: "pending",
    verifier_score: 0.0,
    source_ids: ["raw:objective.md"],
  });

  return runDir;
}

function compileRunDir(runDir) {
  // Prefer the installed `receipts-core` binary from PATH (installed via
  // `cargo install receipts-skill`). Fall back to `cargo run` only when we're
  // inside a source checkout that still has receipts-compiler/Cargo.toml —
  // the npm-distributed package ships schemas + fixtures only, so cargo
  // won't work there even if it's installed.
  const compilerCargoToml = path.join(compilerDir, "Cargo.toml");
  const sourceCheckout = fs.existsSync(compilerCargoToml);

  let result;
  if (sourceCheckout) {
    result = spawnSync(
      "cargo",
      ["run", "--quiet", "--bin", "receipts-core", "--", "compile", "--run-dir", runDir],
      {
        cwd: compilerDir,
        encoding: "utf8",
        shell: process.platform === "win32",
      },
    );
  } else {
    result = spawnSync("receipts-core", ["compile", "--run-dir", runDir], {
      encoding: "utf8",
      shell: process.platform === "win32",
    });
  }

  if (result.error) {
    if (!sourceCheckout && result.error.code === "ENOENT") {
      fail(
        "receipts-core binary not found on PATH. Install it with `cargo install --path receipts-compiler` " +
          "(or run from a source checkout that has receipts-compiler/Cargo.toml).",
      );
    }
    fail(`receipts-compiler spawn failed: ${result.error.message}`);
  }
  const stdout = typeof result.stdout === "string" ? result.stdout.trim() : "";
  const stderr = typeof result.stderr === "string" ? result.stderr.trim() : "";
  if (result.status !== 0) {
    fail(
      [
        `receipts-compiler failed with exit code ${result.status}`,
        stdout,
        stderr,
      ]
        .filter(Boolean)
        .join("\n"),
    );
  }

  // F12: the Rust compiler is the single writer of state/input_fingerprint.json.
  return stdout;
}

function readPacket(runDir) {
  const packetPath = path.join(runDir, "state", "next_pass_packet.json");
  const packet = fs.readFileSync(packetPath, "utf8");
  return { packetPath, packet };
}

function writeCodexPrompt(runDir, packetPath) {
  const promptPath = path.join(runDir, "state", "codex-next-pass.md");
  fs.writeFileSync(
    promptPath,
    [
      "# Receipts Next Pass",
      "",
      "Read the compiled packet below and act as the main synthesis brain.",
      "",
      "Rules:",
      "- Treat packet state as explicit, not latent.",
      "- Drill down into raw sources when the packet says `needs_raw_drilldown` or when confidence is low.",
      "- Append new observations through the run directory, then recompile before the next pass.",
      "",
      `Packet path: ${packetPath}`,
      "",
    ].join("\n"),
    "utf8",
  );
  return promptPath;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const runDir = args.mode === "new-objective" ? createRunDir(args.objective) : args.runDir;
  if (args.mode === "record-synthesis") {
    recordSynthesis(runDir, args.summary);
  }
  const compileOutput = compileRunDir(runDir);
  const { packetPath, packet } = readPacket(runDir);
  const promptPath = writeCodexPrompt(runDir, packetPath);

  process.stderr.write(`${compileOutput}\n`);
  process.stderr.write(`codex_prompt=${promptPath}\n`);
  process.stdout.write(packet);
}

main();
