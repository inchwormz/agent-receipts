#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const compilerDir = path.join(root, "receipts-compiler");
const readinessCargoTarget = path.join(root, ".receipts", "readiness-cargo", `${process.platform}-${process.arch}`);

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? root,
    encoding: "utf8",
    shell: process.platform === "win32",
    env: options.env ? { ...process.env, ...options.env } : process.env,
  });

  if (result.status !== 0) {
    throw new Error(
      [
        `Command failed: ${command} ${args.join(" ")}`,
        `exit=${result.status}`,
        result.stdout.trim(),
        result.stderr.trim(),
      ]
        .filter(Boolean)
        .join("\n"),
    );
  }

  return result;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function safeRemove(target) {
  if (!fs.existsSync(target)) return;
  const resolved = path.resolve(target);
  if (!resolved.startsWith(root)) {
    throw new Error(`Refusing to remove outside receipts root: ${resolved}`);
  }
  fs.rmSync(resolved, { recursive: true, force: true });
}

function appendJsonl(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.appendFileSync(file, `${JSON.stringify(value)}\n`, "utf8");
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

function writeJsonl(file, records) {
  fs.writeFileSync(file, `${records.map((record) => JSON.stringify(record)).join("\n")}\n`, "utf8");
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

function parsePacket(stdout) {
  const packet = JSON.parse(stdout);
  assert(packet.objective_id, "packet missing objective_id");
  assert(Array.isArray(packet.evidence), "packet missing evidence array");
  assert(Array.isArray(packet.halt_signals), "packet missing halt_signals array");
  assert(Array.isArray(packet.sources), "packet missing sources array");
  assert(
    packet.schema_version === "2.0.0",
    `packet schema_version must be "2.0.0" (got ${JSON.stringify(packet.schema_version)})`,
  );
  assertPacketItemsAreSourceBacked(packet);
  assertPacketDeterminism(packet);
  return packet;
}

function assertPacketDeterminism(packet) {
  for (const source of packet.sources ?? []) {
    assert(
      source.hash_alg === "fnv1a-64",
      `packet source ${source.source_id} must declare hash_alg "fnv1a-64" (got ${JSON.stringify(source.hash_alg)})`,
    );
    assert(
      /^[0-9a-f]{16}$/.test(String(source.hash ?? "")),
      `packet source ${source.source_id} hash "${source.hash}" is not a valid fnv1a-64 digest`,
    );
    // G6: forward-slash and absolute-path hygiene now applies to every source
    // kind, not just raw. File/command/test/log refs that leak a machine-
    // specific absolute path make the packet non-portable, and backslashes
    // break determinism across platforms. The ingest normalizer (G1+G10)
    // rewrites absolute-inside-repo file refs to repo-relative up front, so
    // any remaining absolute path here is either an un-normalized agent input
    // or points OUTSIDE the repo — neither is tolerable in a deterministic
    // packet.
    const sourcePath = String(source.path ?? "");
    assert(
      !sourcePath.includes("\\"),
      `packet source ${source.source_id} path must use forward slashes; got "${sourcePath}"`,
    );
    // Absolute-path check — drive-letter (C:\, C:/) OR POSIX-absolute (/...).
    assert(
      !/^[A-Za-z]:[\\/]/.test(sourcePath) && !sourcePath.startsWith("/"),
      `packet source ${source.source_id} path must be run-dir- or repo-relative; got machine-specific "${sourcePath}"`,
    );
    if (source.kind === "raw") {
      assert(
        String(source.observed_at ?? "").length > 0 &&
          source.observed_at !== "raw-ingest" &&
          source.observed_at !== "verifier",
        `packet raw source ${source.source_id} observed_at must not be a placeholder (got ${JSON.stringify(source.observed_at)})`,
      );
    }
  }
}

function assertSourceIds(items, section) {
  for (const item of items ?? []) {
    const id = item.id ?? "<unknown>";
    assert(Array.isArray(item.source_ids), `${section}:${id} missing source_ids`);
    assert(item.source_ids.length > 0, `${section}:${id} has empty source_ids`);
  }
}

function assertPacketItemsAreSourceBacked(packet) {
  assertSourceIds(packet.evidence, "evidence");
  assertSourceIds(packet.trusted_facts, "trusted_facts");
  assertSourceIds(packet.active_hypotheses, "active_hypotheses");
  assertSourceIds(packet.contradictions, "contradictions");
  assertSourceIds(packet.recurring_failure_patterns, "recurring_failure_patterns");
  assertSourceIds(packet.candidate_actions, "candidate_actions");
  assertSourceIds(packet.verifier_findings, "verifier_findings");
  assertSourceIds(packet.halt_signals, "halt_signals");
}

function runDriver(args) {
  return run("node", ["driver.mjs", ...args]);
}

function runExpectFail(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? root,
    encoding: "utf8",
    shell: process.platform === "win32",
    env: options.env ? { ...process.env, ...options.env } : process.env,
  });
  assert(result.status !== 0, `Command unexpectedly passed: ${command} ${args.join(" ")}`);
  return result;
}

// The readiness fixture uses a single synthetic subagent, so the default
// RECEIPTS_MIN_AGENT_COVERAGE floor of 3 would fail. The floor is still exercised
// in real runs; this override keeps the fixture's intent (gate mechanics only).
function runStrictGate(runDir) {
  return run("node", ["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
}

function extractRunDir(stderr) {
  const match = stderr.match(/compiled run_dir=(.*?) snapshot=/);
  assert(match, "driver stderr did not include compiled run_dir");
  return match[1];
}

function checkDriverIsCodexNative() {
  const driver = fs.readFileSync(path.join(root, "driver.mjs"), "utf8");
  assert(!driver.includes("claude -p"), "driver still references claude -p");
  assert(!driver.includes("callClaude"), "driver still contains callClaude");
  assert(!driver.includes('spawn("claude"'), "driver still spawns claude");
}

// Receipts ships two skill surfaces — Claude Code at skills/claude/SKILL.md and
// Codex at skills/codex/SKILL.md. Readiness asserts the Receipts *contract* is
// present in each (subagent fanout, Prime consumption rule, strict gate,
// machine-readable records). Model names, reasoning-effort policy, and other
// environment-specific details live in receipts-agent-policy.json, not the
// contract itself.
// Phrases that must appear in every Receipts skill contract, in either their
// CLI form (`receipts conclude`) or the underlying script form
// (`strict-gate.mjs`). A skill can use either spelling. These are the
// load-bearing doctrine lines - if a skill surface drifts and loses one,
// readiness goes red.
const RECEIPTS_CONTRACT_PATTERNS = [
  { label: "zero-burden briefs", match: /task-only|zero[- ]burden/i },
  { label: "absorb step", match: /receipts absorb|ingest-subagent\.mjs/ },
  { label: "gate step", match: /receipts conclude|receipts gate|strict-gate/ },
  { label: "receipt-minting doctrine", match: /receipts run/ },
  { label: "readiness step", match: /receipts ready|npm run ready/ },
];

function checkSkillRequiresSubagents() {
  const skillFiles = [
    path.join(root, "skills", "claude", "SKILL.md"),
    path.join(root, "skills", "codex", "SKILL.md"),
  ];
  const missing = skillFiles.filter((file) => !fs.existsSync(file));
  assert(
    missing.length === 0,
    `missing skill contract file(s): ${missing.map((file) => path.relative(root, file)).join(", ")}`,
  );

  for (const skillPath of skillFiles) {
    const relName = path.relative(root, skillPath);
    const skill = fs.readFileSync(skillPath, "utf8");
    // Substantive subagent fanout — the skill must teach per-lane work.
    // Legacy surfaces used a "Subagent Lanes" heading; current ones teach
    // the per-lane absorb motion directly.
    assert(
      /Subagent Lanes|[Pp]er lane/.test(skill),
      `${relName} lacks a subagent-lane fanout section`,
    );
    assert(
      skill.includes("Prime consumes recompiled packets, not raw subagent chat") ||
        skill.includes("Prime consumes only the recompiled packet") ||
        skill.includes("never raw subagent chat as ground truth"),
      `${relName} does not block direct Prime consumption of subagent chat`,
    );
    for (const pattern of RECEIPTS_CONTRACT_PATTERNS) {
      assert(
        pattern.match.test(skill),
        `${relName} does not reference the ${pattern.label} (expected ${pattern.match})`,
      );
    }
  }
}

// Agent policy is optional — Receipts does not require any particular model
// choice. When the file is present, enforce only the Receipts contract fields
// (fanout shape + Prime consumption rule). Model names and reasoning-effort
// are environment-specific and may vary by Prime surface.
function checkAgentPolicy() {
  const policyPath = path.join(root, "receipts-agent-policy.json");
  if (!fs.existsSync(policyPath)) return;
  const policy = JSON.parse(fs.readFileSync(policyPath, "utf8"));
  if (policy.fanout) {
    assert(
      Number.isInteger(policy.fanout.micro_lanes) && policy.fanout.micro_lanes > 0,
      "agent policy fanout.micro_lanes must be a positive integer",
    );
    assert(
      Number.isInteger(policy.fanout.broad_lanes_max) && policy.fanout.broad_lanes_max >= 0,
      "agent policy fanout.broad_lanes_max must be a non-negative integer",
    );
  }
  if (policy.prime_consumption) {
    assert(
      policy.prime_consumption.raw_subagent_chat_allowed === false,
      "agent policy must forbid raw subagent chat (prime_consumption.raw_subagent_chat_allowed=false)",
    );
  }
}

function addSyntheticSubagentEvidence(runDir) {
  const fixturePath = path.join(runDir, "raw", "subagents", "readiness-subagent-output.md");
  fs.mkdirSync(path.dirname(fixturePath), { recursive: true });
  // The root-cause record cites raw:subagents/... so it overlaps with the
  // auto subagent-session record's source_ids and passes R5 traceability.
  const rawSubagentSourceId = "raw:subagents/readiness-subagent-output.md";
  fs.writeFileSync(
    fixturePath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-root-cause-readiness",
        kind: "root-cause",
        summary: "Readiness root-cause claim intentionally starts without direct provenance.",
        source_ids: ["raw:objective.md", rawSubagentSourceId],
        observed_at: new Date().toISOString(),
      }),
      "```",
      "",
      "```receipts-verifier-jsonl",
      JSON.stringify({
        id: "vf-subagent-fanout-readiness",
        summary: "Readiness subagent fixture was ingested through quarantine parser.",
        status: "passed",
        verifier_score: 0.9,
        source_ids: ["raw:objective.md"],
        finding_kind: "bootstrap",
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  run("node", [
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "readiness",
    "--agent-id",
    "synthetic",
    "--from",
    fixturePath,
  ]);
}

function addDirectProvenanceToReadinessRootCause(runDir) {
  const evidencePath = path.join(runDir, "worker-results", "evidence.jsonl");
  const sourceId = "file:scripts/readiness.mjs:root-cause-fixture";
  const sourcePath = path.join(root, "scripts", "readiness.mjs");
  // Keep the raw:subagents/* source_id so the record remains traceable back to
  // the subagent session that produced it (R5).
  const rawSubagentSourceId = "raw:subagents/readiness-subagent-output.md";
  const records = readJsonl(evidencePath).map((record) => {
    if (record.id !== "ev-root-cause-readiness") return record;
    return {
      ...record,
      source_ids: [sourceId, "raw:objective.md", rawSubagentSourceId],
      source_refs: [
        {
          source_id: sourceId,
          path: "scripts/readiness.mjs",
          kind: "file",
          hash: fnv1aHash(fs.readFileSync(sourcePath)),
          span: "1",
          observed_at: new Date().toISOString(),
        },
      ],
    };
  });
  writeJsonl(evidencePath, records);
}

function addBareDirectSourceIdToReadinessRootCause(runDir) {
  const evidencePath = path.join(runDir, "worker-results", "evidence.jsonl");
  const sourceId = "file:scripts/readiness.mjs:bare-direct-fixture";
  const records = readJsonl(evidencePath).map((record) => {
    if (record.id !== "ev-root-cause-readiness") return record;
    return {
      ...record,
      source_ids: [sourceId, ...record.source_ids.filter((id) => id !== sourceId)],
      source_refs: [],
    };
  });
  writeJsonl(evidencePath, records);
}

function checkStrictGate() {
  const generated = runDriver(["strict gate readiness objective"]);
  parsePacket(generated.stdout);
  const generatedRunDir = extractRunDir(generated.stderr);

  const initialGate = runExpectFail("node", [
    "scripts/strict-gate.mjs",
    "--run-dir",
    generatedRunDir,
  ]);
  assert(
    initialGate.stdout.includes("pass-0001") || initialGate.stdout.includes("only objective evidence"),
    "strict gate failure did not explain initial run failure",
  );
  const bareReturnedProse = path.join(generatedRunDir, "raw", "subagents", "bare-returned-prose.md");
  fs.mkdirSync(path.dirname(bareReturnedProse), { recursive: true });
  fs.writeFileSync(bareReturnedProse, "Bare returned subagent prose without machine records.\n", "utf8");
  const bareReturnedGate = runExpectFail("node", [
    "scripts/strict-gate.mjs",
    "--run-dir",
    generatedRunDir,
  ]);
  assert(
    bareReturnedGate.stdout.includes("no subagent-session evidence records"),
    "bare returned subagent prose satisfied the gate without mechanical ingestion",
  );

  // Forgiving ingest (post-mortem 2026-07-12): prose-only output is captured
  // as a demoted `unstructured` record instead of crashing the lane. Assert
  // the fallback fires AND stays non-substantive (demotion warning present).
  // Runs AFTER the bare-prose gate probe above, because this ingest creates
  // the run's first subagent-session record.
  const proseOnlySubagent = path.join(generatedRunDir, "raw", "prose-only-subagent.md");
  fs.writeFileSync(proseOnlySubagent, "This is a prose-only subagent response with no machine records.\n", "utf8");
  const proseOnlyIngest = run("node", [
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    generatedRunDir,
    "--lane",
    "readiness-prose-only",
    "--agent-id",
    "synthetic-prose",
    "--from",
    proseOnlySubagent,
  ]);
  const proseReport = JSON.parse(proseOnlyIngest.stdout);
  assert(proseReport.unstructured === true, "prose-only ingest must flag unstructured fallback");
  const proseRecords = readJsonl(path.join(generatedRunDir, "worker-results", "evidence.jsonl"));
  const unstructuredRecord = proseRecords.find((record) => record.kind === "unstructured");
  assert(unstructuredRecord, "prose-only ingest must synthesize an unstructured record");
  assert(
    Array.isArray(unstructuredRecord.provenance_warnings) && unstructuredRecord.provenance_warnings.length > 0,
    "unstructured record must carry a demotion warning",
  );

  addSyntheticSubagentEvidence(generatedRunDir);
  const staleSynthesis = runExpectFail("node", [
    "driver.mjs",
    "--run-dir",
    generatedRunDir,
    "--record-synthesis",
    "this synthesis must fail until the subagent JSONL has been recompiled",
  ]);
  assert(
    staleSynthesis.stderr.includes("next_pass_packet.json is stale"),
    "record-synthesis did not require recompile after subagent ingestion",
  );
  runDriver(["--run-dir", generatedRunDir]);
  const synthesis = runDriver([
    "--run-dir",
    generatedRunDir,
    "--record-synthesis",
    "readiness source-backed synthesis consumed promoted subagent evidence",
  ]);
  const synthesisPacket = parsePacket(synthesis.stdout);
  assert(synthesisPacket.pass_id === "pass-0002", "strict-gate fixture did not advance pass id");
  assert(
    synthesisPacket.halt_signals.some((signal) => signal.kind === "ready-to-halt"),
    "strict-gate fixture did not compile to ready-to-halt",
  );

  const summaryOnlyGate = runExpectFail("node", [
    "scripts/strict-gate.mjs",
    "--run-dir",
    generatedRunDir,
  ]);
  assert(
    summaryOnlyGate.stdout.includes("summary-only evidence"),
    "strict gate did not reject summary-only substantive evidence",
  );

  addBareDirectSourceIdToReadinessRootCause(generatedRunDir);
  const bareDirectCompile = runExpectFail("node", [
    "driver.mjs",
    "--run-dir",
    generatedRunDir,
  ]);
  assert(
    bareDirectCompile.stderr.includes("references unknown artifact"),
    "compiler did not reject bare direct source_ids without source_refs",
  );

  addDirectProvenanceToReadinessRootCause(generatedRunDir);
  runDriver(["--run-dir", generatedRunDir]);

  const gate = runStrictGate(generatedRunDir);
  const gateReport = JSON.parse(gate.stdout);
  assert(gateReport.ok === true, "strict gate did not pass promoted fixture");

  // Tamper with a quarantined raw subagent artifact after ingest and prove the
  // strict gate catches it via the new raw-hash verification path. This keeps
  // quarantined evidence immutable — the deterministic guarantee Prime relies
  // on when consuming the recompiled packet.
  const subagentDir = path.join(generatedRunDir, "raw", "subagents");
  const subagentFiles = fs.existsSync(subagentDir)
    ? fs.readdirSync(subagentDir).map((name) => path.join(subagentDir, name))
    : [];
  assert(subagentFiles.length > 0, "tamper probe needs a quarantined subagent artifact");
  const tamperTarget = subagentFiles[0];
  const original = fs.readFileSync(tamperTarget);
  fs.appendFileSync(tamperTarget, "\n# tampered after ingest\n", "utf8");
  const tamperedGate = runExpectFail("node", [
    "scripts/strict-gate.mjs",
    "--run-dir",
    generatedRunDir,
  ]);
  assert(
    tamperedGate.stdout.includes("hash mismatch"),
    "strict gate did not detect tampered raw subagent artifact",
  );
  fs.writeFileSync(tamperTarget, original);

  safeRemove(generatedRunDir);
}

function main() {
  checkDriverIsCodexNative();
  checkSkillRequiresSubagents();
  checkAgentPolicy();
  checkStrictGate();

  // The npm package includes the engine source. Pin Cargo to this repository's
  // ignored target so a same-named crate from another checkout cannot supply a
  // stale false green through a machine-wide CARGO_TARGET_DIR.
  const cargoTomlPath = path.join(compilerDir, "Cargo.toml");
  assert(fs.existsSync(cargoTomlPath), `bundled engine source is missing: ${cargoTomlPath}`);
  run("cargo", ["fmt", "--manifest-path", cargoTomlPath, "--check"]);
  run("cargo", ["test", "--locked", "--manifest-path", cargoTomlPath], {
    env: { CARGO_TARGET_DIR: readinessCargoTarget },
  });

  const fixtureState = path.join(compilerDir, "tests", "fixtures", "run-basic", "state");
  safeRemove(fixtureState);
  const fixture = runDriver(["--run-dir", path.join(compilerDir, "tests", "fixtures", "run-basic")]);
  const fixturePacket = parsePacket(fixture.stdout);
  assert(fixturePacket.evidence.length === 2, "fixture packet should contain two evidence records");
  safeRemove(fixtureState);

  const generated = runDriver(["readiness check objective"]);
  const generatedPacket = parsePacket(generated.stdout);
  assert(
    generatedPacket.objective === "readiness check objective",
    "generated packet objective mismatch",
  );
  const generatedRunDir = extractRunDir(generated.stderr);
  const synthesis = runDriver([
    "--run-dir",
    generatedRunDir,
    "--record-synthesis",
    "readiness synthesis consumed packet state",
  ]);
  const synthesisPacket = parsePacket(synthesis.stdout);
  assert(synthesisPacket.pass_id === "pass-0002", "record-synthesis did not advance pass id");
  assert(
    synthesisPacket.evidence.some((item) => item.kind === "codex-synthesis"),
    "record-synthesis evidence missing",
  );
  assert(
    synthesisPacket.verifier_findings.every((finding) => finding.status === "passed"),
    "record-synthesis did not clear verifier findings",
  );
  safeRemove(generatedRunDir);

  console.log("receipts readiness: passed");
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
