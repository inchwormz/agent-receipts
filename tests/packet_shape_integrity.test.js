// Round-trip integrity tests for the Receipts packet shape.
//
// Covers:
// - R1/R2/R3 — agent_id, lane, confidence, rationale, diff_ref survive ingest
//   (written directly on fenced JSONL records) and compile through driver.mjs
//   into state/next_pass_packet.json.
// - R4/R9 — a subagent report containing only a `BLOCKED <reason>` sentinel is
//   accepted by the ingester (exit 0) and produces a `kind:"blocker"` evidence
//   record.
//
// Each test creates its own ephemeral run dir under .codex/receipts/tmp-test-*/
// and cleans up via t.after.
import { strict as assert } from "node:assert";
import { spawn, spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const thisFile = fileURLToPath(import.meta.url);
const repoRoot = path.dirname(path.dirname(thisFile));

function freshRunDir(name) {
  const stamp = new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
  const pid = process.pid;
  const runDir = path.join(
    repoRoot,
    ".codex",
    "receipts-core",
    `tmp-test-${stamp}-${pid}-${name}`,
  );
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: `obj-test-${stamp}`,
        run_id: `run-test-${stamp}`,
        branch_id: "main",
        pass_id: "pass-0001",
        created_at: new Date().toISOString(),
        repo_root: repoRoot,
      },
      null,
      2,
    ) + "\n",
    "utf8",
  );
  fs.writeFileSync(path.join(runDir, "task.md"), `Test objective for ${name}\n`, "utf8");
  // Seed one raw artifact so the compiler has a source to anchor evidence to.
  fs.writeFileSync(
    path.join(runDir, "raw", "objective.md"),
    `# Test Objective\n\n${name}\n`,
    "utf8",
  );
  // Seed an objective evidence record so the evidence pipeline is non-empty.
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-objective",
      kind: "objective",
      summary: `Test objective for ${name}`,
      source_ids: ["raw:objective.md"],
      observed_at: new Date().toISOString(),
    }) + "\n",
    "utf8",
  );
  fs.writeFileSync(
    path.join(runDir, "verifier-results", "findings.jsonl"),
    JSON.stringify({
      id: "vf-codex-synthesis-pending",
      summary: "Codex synthesis has not consumed this packet yet",
      status: "pending",
      verifier_score: 0.0,
      source_ids: ["raw:objective.md"],
    }) + "\n",
    "utf8",
  );
  return runDir;
}

function removeDir(dir) {
  if (!dir) return;
  if (!dir.startsWith(path.join(repoRoot, ".codex", "receipts"))) {
    throw new Error(`refusing to remove outside .codex/receipts: ${dir}`);
  }
  fs.rmSync(dir, { recursive: true, force: true });
}

function runNode(args, options = {}) {
  const result = spawnSync("node", args, {
    cwd: repoRoot,
    encoding: "utf8",
    shell: process.platform === "win32",
    ...options,
  });
  return result;
}

function readJsonl(file) {
  return fs
    .readFileSync(file, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

test("F4 ingest: caller attribution WINS; record-declared identity survives only as claimed_*", (t) => {
  const runDir = freshRunDir("ingest-fields");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "fixture-fields.md");
  const observedAt = new Date().toISOString();
  const evidenceRecord = {
    id: "ev-fixture-fields",
    kind: "code-change",
    summary: "Patched fixture file to demonstrate field passthrough",
    source_ids: ["raw:objective.md"],
    observed_at: observedAt,
    // F4: a record may CLAIM any identity it likes — the caller's stamp wins,
    // and the claim is preserved for audit as claimed_agent_id/claimed_lane.
    agent_id: "fixture-worker-alpha",
    lane: "fixture-lane",
    confidence: 0.92,
    rationale: "Deterministic test of field passthrough",
    diff_ref: "test-diff-ref-12345",
    span_before: "old span",
    span_after: "new span",
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(evidenceRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "caller-lane-wins",
    "--agent-id",
    "caller-agent-wins",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `ingest exited non-zero: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const fixture = evidence.find((record) => record.id === "ev-fixture-fields");
  assert.ok(fixture, "fixture evidence record must be present after ingest");
  assert.equal(fixture.agent_id, "caller-agent-wins", "caller agent stamp must win");
  assert.equal(fixture.lane, "caller-lane-wins", "caller lane stamp must win");
  assert.equal(fixture.claimed_agent_id, "fixture-worker-alpha", "claimed identity preserved");
  assert.equal(fixture.claimed_lane, "fixture-lane", "claimed lane preserved");
  assert.equal(fixture.confidence, 0.92);
  assert.equal(fixture.rationale, "Deterministic test of field passthrough");
  assert.equal(fixture.diff_ref, "test-diff-ref-12345");
  assert.equal(fixture.span_before, "old span");
  assert.equal(fixture.span_after, "new span");
});

test("F1+F4 driver: caller attribution reaches the packet; unverified evidence is NOT a trusted fact", (t) => {
  const runDir = freshRunDir("driver-fields");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "fixture-driver.md");
  const observedAt = new Date().toISOString();
  const evidenceRecord = {
    id: "ev-driver-fixture",
    kind: "observation",
    summary: "Driver test observation carrying attribution and confidence",
    source_ids: ["raw:objective.md"],
    observed_at: observedAt,
    agent_id: "driver-worker",
    lane: "driver-lane",
    confidence: 0.81,
    rationale: "Attribution should reach the compiled packet",
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(evidenceRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "driver-lane-caller",
    "--agent-id",
    "driver-agent-caller",
    "--from",
    subagentPath,
  ]);
  assert.equal(ingest.status, 0, `ingest failed: ${ingest.stderr}`);

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(
    driver.status,
    0,
    `driver failed: stdout=${driver.stdout}\nstderr=${driver.stderr}`,
  );

  const packet = JSON.parse(
    fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"),
  );
  const compiled = packet.evidence.find((record) => record.id === "ev-driver-fixture");
  assert.ok(compiled, "compiled packet must contain driver fixture evidence");
  assert.equal(compiled.agent_id, "driver-agent-caller", "caller stamp must reach the packet");
  assert.equal(compiled.lane, "driver-lane-caller");
  assert.equal(compiled.claimed_agent_id, "driver-worker", "claimed identity must reach the packet");
  assert.equal(compiled.confidence, 0.81);
  assert.equal(compiled.rationale, "Attribution should reach the compiled packet");

  // F1: no passed verifier finding backs this observation, so it must stay
  // OUT of trusted_facts — self-graded confidence is not trust.
  const fact = packet.trusted_facts.find((item) => item.id === "fact:ev-driver-fixture");
  assert.equal(
    fact,
    undefined,
    `unverified evidence must not be promoted to trusted_facts; got ${JSON.stringify(fact)}`,
  );
});

// Exposed from ingest-subagent.mjs for parity with the test harness.
function fnv1aHashString(input) {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  const bytes = Buffer.from(String(input), "utf8");
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

function fnv1aHashBytes(buffer) {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const byte of buffer) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

test("H1 ingest: non-file source_refs with placeholder hash get fnv1a-hashed from source_id", (t) => {
  const runDir = freshRunDir("h1-auto-hash");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "h1-fixture.md");
  const verifierRecord = {
    id: "vf-h1-cargo-fixture",
    summary: "cargo test reports green",
    status: "passed",
    verifier_score: 1.0,
    source_ids: ["test:foo/bar", "raw:objective.md"],
    source_refs: [
      {
        source_id: "test:foo/bar",
        path: "test/foo/bar",
        kind: "test",
        hash: "fill-with-hash-or-stable-placeholder",
        hash_alg: "fnv1a-64",
        span: null,
        observed_at: new Date().toISOString(),
      },
    ],
    closure_reason: "actual-run",
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-verifier-jsonl",
      JSON.stringify(verifierRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "h1-lane",
    "--agent-id",
    "h1-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `H1 ingest must exit 0: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const findings = readJsonl(path.join(runDir, "verifier-results", "findings.jsonl"));
  const fixture = findings.find((record) => record.id === "vf-h1-cargo-fixture");
  assert.ok(fixture, "H1 fixture finding must be present after ingest");
  const ref = (fixture.source_refs ?? []).find((source) => source.source_id === "test:foo/bar");
  assert.ok(ref, "H1 finding must keep the test: source_ref");
  assert.equal(
    ref.hash,
    fnv1aHashString("test:foo/bar"),
    `H1 hash must match fnv1a(source_id); got ${ref.hash}`,
  );
  // F2 truth-in-labeling: a digest derived from the claim's own name is an
  // identity key, not provenance — it must be stamped hash_basis:"label".
  assert.equal(
    ref.hash_basis,
    "label",
    `H1 label-derived hash must be stamped hash_basis:"label"; got ${ref.hash_basis}`,
  );
});

test("H2 ingest: file source_refs with out-of-range spans get clipped to actual line count", (t) => {
  const runDir = freshRunDir("h2-clip-span");
  t.after(() => removeDir(runDir));

  // package.json is a known-small file (17 lines). Span 999-1000 must clip.
  const filePath = path.join(repoRoot, "package.json");
  const lineCount = fs.readFileSync(filePath, "utf8").split(/\r?\n/).length;
  assert.ok(lineCount < 999, `test assumes package.json has <<999 lines, got ${lineCount}`);

  const subagentPath = path.join(runDir, "raw", "subagents", "h2-fixture.md");
  const evidenceRecord = {
    id: "ev-h2-clip",
    kind: "observation",
    summary: "Reference package.json line that doesn't exist",
    source_ids: ["file:package.json:999-1000", "raw:objective.md"],
    source_refs: [
      {
        source_id: "file:package.json:999-1000",
        path: "package.json",
        kind: "file",
        hash: "placeholder-will-be-filled",
        hash_alg: "fnv1a-64",
        span: "999-1000",
        observed_at: new Date().toISOString(),
      },
    ],
    observed_at: new Date().toISOString(),
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(evidenceRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "h2-lane",
    "--agent-id",
    "h2-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `H2 ingest must exit 0 (span should clip, not fail): stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const fixture = evidence.find((record) => record.id === "ev-h2-clip");
  assert.ok(fixture, "H2 fixture evidence must be present");
  const ref = (fixture.source_refs ?? []).find(
    (source) => source.source_id === "file:package.json:999-1000",
  );
  assert.ok(ref, "H2 finding must keep the file source_ref");
  const match = /^(\d+)(?:-(\d+))?$/.exec(String(ref.span));
  assert.ok(match, `H2 span must still be a valid line/range; got ${ref.span}`);
  const end = Number(match[2] ?? match[1]);
  assert.ok(
    end <= lineCount,
    `H2 span "${ref.span}" must be clipped to <= ${lineCount}; saw end=${end}`,
  );
  // F7: a repaired citation is a demoted record — the clip must be recorded
  // as a provenance warning, which bars the record from trusted_facts.
  assert.ok(
    Array.isArray(fixture.provenance_warnings) &&
      fixture.provenance_warnings.some((w) => String(w).startsWith("span-clipped:")),
    `H2 clipped record must carry a span-clipped provenance warning; got ${JSON.stringify(fixture.provenance_warnings)}`,
  );
});

test("H3 ingest: direct file: source_id with no source_refs gets synthesized with on-disk hash", (t) => {
  const runDir = freshRunDir("h3-synth-ref");
  t.after(() => removeDir(runDir));

  // readiness.mjs is a stable repo file with plenty of lines.
  const filePath = path.join(repoRoot, "scripts", "readiness.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));

  const subagentPath = path.join(runDir, "raw", "subagents", "h3-fixture.md");
  const evidenceRecord = {
    id: "ev-h3-synth",
    kind: "observation",
    summary: "Direct source id with no explicit source_refs block",
    // Note: NO source_refs — ingest should synthesize one.
    source_ids: ["file:scripts/readiness.mjs:1", "raw:objective.md"],
    observed_at: new Date().toISOString(),
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(evidenceRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "h3-lane",
    "--agent-id",
    "h3-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `H3 ingest must exit 0 when source_refs are auto-synthesized: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const fixture = evidence.find((record) => record.id === "ev-h3-synth");
  assert.ok(fixture, "H3 fixture evidence must be present");
  assert.ok(
    Array.isArray(fixture.source_refs) && fixture.source_refs.length > 0,
    "H3 ingest must synthesize source_refs for direct source_ids",
  );
  const synth = fixture.source_refs.find(
    (source) => source.source_id === "file:scripts/readiness.mjs:1",
  );
  assert.ok(synth, "H3 synthesized ref must carry the original direct source_id");
  assert.equal(synth.kind, "file");
  assert.equal(synth.path, "scripts/readiness.mjs");
  assert.equal(
    synth.hash,
    expectedHash,
    `H3 synthesized hash must equal fnv1a(file bytes); got ${synth.hash}`,
  );
});

test("H4 ingest: evidence without raw:subagents/* source_id gets auto-stamped", (t) => {
  const runDir = freshRunDir("h4-auto-stamp");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "h4-fixture.md");
  const evidenceRecord = {
    id: "ev-h4-stamp",
    kind: "observation",
    summary: "Record that forgot to cite its own raw subagent path",
    // Deliberately no raw:subagents/* id — H4 must inject it.
    source_ids: ["raw:objective.md"],
    observed_at: new Date().toISOString(),
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(evidenceRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "h4-lane",
    "--agent-id",
    "h4-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `H4 ingest must exit 0: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const fixture = evidence.find((record) => record.id === "ev-h4-stamp");
  assert.ok(fixture, "H4 fixture evidence must be present");
  assert.ok(
    (fixture.source_ids ?? []).some(
      (id) => typeof id === "string" && id.startsWith("raw:subagents/"),
    ),
    `H4 must auto-stamp raw:subagents/* source_id; saw ${JSON.stringify(fixture.source_ids)}`,
  );
});

test("H6 ingest: verifier closure_reason survives ingest -> driver -> packet -> strict-gate", (t) => {
  const runDir = freshRunDir("h6-closure-reason");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "h6-fixture.md");
  // Also provide a file: ref so agent coverage is satisfied downstream.
  const filePath = path.join(repoRoot, "scripts", "readiness.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));
  const observedAt = new Date().toISOString();
  const evidenceRecord = {
    id: "ev-h6-anchor",
    kind: "code-change",
    summary: "Anchors bounded-audit closure evidence to a stable file",
    source_ids: ["file:scripts/readiness.mjs:1", "raw:objective.md"],
    source_refs: [
      {
        source_id: "file:scripts/readiness.mjs:1",
        path: "scripts/readiness.mjs",
        kind: "file",
        hash: expectedHash,
        hash_alg: "fnv1a-64",
        span: "1",
        observed_at: observedAt,
      },
    ],
    observed_at: observedAt,
  };
  const verifierRecord = {
    id: "vf-h6-bounded-closure",
    summary: "Bounded-audit closure with explicit closure_reason stamp",
    status: "passed",
    verifier_score: 1.0,
    source_ids: ["raw:objective.md"],
    closure_reason: "bounded-audit",
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(evidenceRecord),
      "```",
      "",
      "```receipts-verifier-jsonl",
      JSON.stringify(verifierRecord),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "h6-lane",
    "--agent-id",
    "h6-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `H6 ingest must exit 0: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  // Verify closure_reason survived ingest.
  const findings = readJsonl(path.join(runDir, "verifier-results", "findings.jsonl"));
  const ingested = findings.find((record) => record.id === "vf-h6-bounded-closure");
  assert.ok(ingested, "H6 verifier must be present after ingest");
  assert.equal(ingested.closure_reason, "bounded-audit");

  // Verify closure_reason survived driver/compile.
  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(
    driver.status,
    0,
    `H6 driver must exit 0: stdout=${driver.stdout}\nstderr=${driver.stderr}`,
  );
  const packet = JSON.parse(
    fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"),
  );
  const compiled = packet.verifier_findings.find(
    (record) => record.id === "vf-h6-bounded-closure",
  );
  assert.ok(compiled, "H6 verifier finding must be in compiled packet");
  assert.equal(
    compiled.closure_reason,
    "bounded-audit",
    "H6 closure_reason must survive through the Rust compiler into packet.verifier_findings",
  );
});

test("ingest: BLOCKED sentinel alone emits a blocker evidence record and exits 0", (t) => {
  const runDir = freshRunDir("blocked-sentinel");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "blocked.md");
  fs.writeFileSync(
    subagentPath,
    [
      "# Subagent Session",
      "",
      "I tried to open the feature flag console but the credentials are not available in this environment.",
      "",
      "BLOCKED feature-flag-unavailable",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "blocked-lane",
    "--agent-id",
    "blocked-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `BLOCKED-only ingest must exit 0: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const blocker = evidence.find((record) => record.kind === "blocker");
  assert.ok(blocker, "blocker evidence record must be emitted for BLOCKED sentinel");
  assert.match(
    blocker.summary,
    /BLOCKED: feature-flag-unavailable/,
    `blocker summary should echo BLOCKED reason, got: ${blocker.summary}`,
  );
  assert.equal(blocker.agent_id, "blocked-agent");
  assert.equal(blocker.lane, "blocked-lane");
  assert.ok(
    Array.isArray(blocker.source_ids) && blocker.source_ids.length > 0,
    "blocker record must carry at least one source_id",
  );
  assert.ok(
    blocker.source_ids.some((id) => typeof id === "string" && id.startsWith("raw:subagents/")),
    "blocker source_ids must include the quarantined raw subagent path",
  );
});

test("ingest: fenced scoreboard BLOCKED line does not emit a blocker", (t) => {
  const runDir = freshRunDir("fenced-scoreboard-blocked");
  t.after(() => removeDir(runDir));
  const subagentPath = path.join(runDir, "raw", "subagents", "scoreboard.md");
  fs.writeFileSync(
    subagentPath,
    [
      "# Successful finalizer",
      "",
      "```text",
      "PASS 242 -> 252",
      "REVISE 192 -> 182",
      "BLOCKED 1377 -> 1377",
      "```",
      "",
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-finalizer-complete",
        kind: "observation",
        summary: "finalizer completed successfully",
        source_ids: ["raw:objective.md"],
        observed_at: "2026-07-13T00:00:00.000Z",
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  const ingest = runNode([
    "scripts/ingest-subagent.mjs", "--run-dir", runDir, "--lane", "scoreboard-lane",
    "--agent-id", "scoreboard-agent", "--from", subagentPath,
  ]);
  assert.equal(ingest.status, 0, `stdout=${ingest.stdout}\nstderr=${ingest.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  assert.ok(evidence.some((record) => record.id === "ev-finalizer-complete"));
  assert.equal(evidence.some((record) => record.kind === "blocker"), false);
});

test("ingest: unfenced numeric BLOCKED transition is scoreboard data, not a halt", (t) => {
  const runDir = freshRunDir("numeric-scoreboard-blocked");
  t.after(() => removeDir(runDir));
  const subagentPath = path.join(runDir, "raw", "subagents", "numeric-scoreboard.md");
  fs.writeFileSync(
    subagentPath,
    [
      "BLOCKED 10 -> 9",
      "",
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-board-counted",
        kind: "observation",
        summary: "board counts were measured",
        source_ids: ["raw:objective.md"],
        observed_at: "2026-07-13T00:00:00.000Z",
      }),
      "```",
    ].join("\n"),
    "utf8",
  );
  const ingest = runNode([
    "scripts/ingest-subagent.mjs", "--run-dir", runDir, "--lane", "numeric-lane",
    "--agent-id", "numeric-agent", "--from", subagentPath,
  ]);
  assert.equal(ingest.status, 0, `stdout=${ingest.stdout}\nstderr=${ingest.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  assert.equal(evidence.some((record) => record.kind === "blocker"), false);
});

// Synthetic-packet helpers for the contradiction tests (G4/G5/G7). We bypass
// the driver and write evidence.jsonl directly so we can control both sides
// of the proposed pair, then run the Rust compiler via `driver.mjs` to
// exercise the real detect_auto_contradictions path.
function writeFullEvidence(runDir, records) {
  const evidencePath = path.join(runDir, "worker-results", "evidence.jsonl");
  fs.writeFileSync(
    evidencePath,
    records.map((record) => JSON.stringify(record)).join("\n") + "\n",
    "utf8",
  );
}

test("G1+G10 ingest: absolute-inside-repo file: source_ids get rewritten to repo-relative", (t) => {
  const runDir = freshRunDir("g1-g10-path-normalize");
  t.after(() => removeDir(runDir));

  // Two subagents cite the SAME physical file with DIFFERENT path forms —
  // one absolute, one repo-relative. After ingest, both records must
  // reference the same normalized source_id so downstream dedupe and
  // contradiction detection can actually line them up.
  const absolutePath = path.join(repoRoot, "driver.mjs").replace(/\\/g, "/");
  const absoluteSourceId = `file:${absolutePath}:1`;
  const repoRelativeSourceId = "file:driver.mjs:1";

  const subagentA = path.join(runDir, "raw", "subagents", "g1-abs.md");
  const subagentB = path.join(runDir, "raw", "subagents", "g1-rel.md");
  fs.writeFileSync(
    subagentA,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-g1-abs",
        kind: "observation",
        summary: "Cites driver.mjs using an absolute filesystem path",
        source_ids: [absoluteSourceId, "raw:objective.md"],
        observed_at: new Date().toISOString(),
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  fs.writeFileSync(
    subagentB,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-g1-rel",
        kind: "observation",
        summary: "Cites driver.mjs using a repo-relative path",
        source_ids: [repoRelativeSourceId, "raw:objective.md"],
        observed_at: new Date().toISOString(),
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingestA = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "g1-lane-a",
    "--agent-id",
    "g1-agent-a",
    "--from",
    subagentA,
  ]);
  assert.equal(
    ingestA.status,
    0,
    `G1 absolute-path ingest must exit 0: stdout=${ingestA.stdout}\nstderr=${ingestA.stderr}`,
  );
  const ingestB = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "g1-lane-b",
    "--agent-id",
    "g1-agent-b",
    "--from",
    subagentB,
  ]);
  assert.equal(
    ingestB.status,
    0,
    `G1 repo-relative ingest must exit 0: stdout=${ingestB.stdout}\nstderr=${ingestB.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const recordA = evidence.find((record) => record.id === "ev-g1-abs");
  const recordB = evidence.find((record) => record.id === "ev-g1-rel");
  assert.ok(recordA, "G1 absolute-form record must survive ingest");
  assert.ok(recordB, "G1 repo-relative record must survive ingest");

  // Both records must cite THE SAME normalized source_id after ingest.
  const sharedFileIds = recordA.source_ids.filter((id) =>
    typeof id === "string" && id.startsWith("file:"),
  );
  const relIds = recordB.source_ids.filter((id) =>
    typeof id === "string" && id.startsWith("file:"),
  );
  assert.ok(sharedFileIds.length > 0, "G1 absolute record must keep a file: source_id");
  assert.ok(relIds.length > 0, "G1 relative record must keep a file: source_id");
  assert.equal(
    sharedFileIds[0],
    repoRelativeSourceId,
    `G1 absolute form ${absoluteSourceId} must be normalized to ${repoRelativeSourceId}; got ${sharedFileIds[0]}`,
  );
  assert.equal(
    relIds[0],
    repoRelativeSourceId,
    "G1 repo-relative form must pass through unchanged",
  );
});

test("G2 strict-gate: subagent-session records without agent_id/lane fail the gate", (t) => {
  const runDir = freshRunDir("g2-session-attribution");
  t.after(() => removeDir(runDir));

  // Craft a manifest stamped to 1.1.0 so G2 enforcement kicks in.
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: "obj-g2",
        run_id: "run-g2",
        branch_id: "main",
        pass_id: "pass-0002",
        created_at: new Date().toISOString(),
      },
      null,
      2,
    ) + "\n",
    "utf8",
  );

  // Seed a broken subagent-session evidence record — no agent_id, no lane.
  // The record also needs to match the raw:subagents/* it cites, so write a
  // matching raw file AND stage evidence.jsonl directly.
  const rawFile = path.join(runDir, "raw", "subagents", "g2-fake.md");
  fs.writeFileSync(rawFile, "# fake session\n", "utf8");
  const sessionId = "raw:subagents/g2-fake.md";
  const observedAt = new Date().toISOString();
  const sessionRecord = {
    id: "ev-subagent-session-g2-fake",
    kind: "subagent-session",
    summary: "Fake subagent session with missing attribution",
    source_ids: [sessionId],
    source_refs: [
      {
        source_id: sessionId,
        path: "raw/subagents/g2-fake.md",
        kind: "raw",
        hash: fnv1aHashBytes(fs.readFileSync(rawFile)),
        hash_alg: "fnv1a-64",
        span: null,
        observed_at: observedAt,
      },
    ],
    observed_at: observedAt,
    // Deliberately omit agent_id + lane.
  };
  writeFullEvidence(runDir, [
    {
      id: "ev-objective",
      kind: "objective",
      summary: "G2 test objective",
      source_ids: ["raw:objective.md"],
      observed_at: observedAt,
    },
    sessionRecord,
  ]);

  // Synth packet the way strict-gate expects it (cheat-compile via driver).
  runNode(["driver.mjs", "--run-dir", runDir]);

  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(
    gate.status,
    0,
    `G2 gate must reject sessions without agent_id/lane: stdout=${gate.stdout}`,
  );
  assert.ok(
    gate.stdout.includes("G2:") || gate.stdout.includes("agent_id AND lane"),
    `G2 gate error must mention the constraint; got stdout=${gate.stdout}`,
  );
});

test("G4 contradictions: observation+proposal pair does not fire a contradiction", (t) => {
  const runDir = freshRunDir("g4-kind-pair-exclusion");
  t.after(() => removeDir(runDir));

  const filePath = path.join(repoRoot, "scripts", "readiness.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));
  const observedAt = new Date().toISOString();
  const baseRef = {
    source_id: "file:scripts/readiness.mjs:1",
    path: "scripts/readiness.mjs",
    kind: "file",
    hash: expectedHash,
    hash_alg: "fnv1a-64",
    span: "1",
    observed_at: observedAt,
  };
  writeFullEvidence(runDir, [
    {
      id: "ev-objective",
      kind: "objective",
      summary: "G4 test objective",
      source_ids: ["raw:objective.md"],
      observed_at: observedAt,
    },
    {
      id: "ev-g4-observation",
      kind: "observation",
      summary: "The check currently fires on all source kinds, not just raw",
      source_ids: [baseRef.source_id, "raw:objective.md"],
      source_refs: [baseRef],
      observed_at: observedAt,
      agent_id: "g4-agent-a",
      lane: "g4-lane-a",
    },
    {
      id: "ev-g4-proposal",
      kind: "proposal",
      summary: "Loosen the check to tolerate absolute paths outside repo_root",
      source_ids: [baseRef.source_id, "raw:objective.md"],
      source_refs: [baseRef],
      observed_at: observedAt,
      agent_id: "g4-agent-b",
      lane: "g4-lane-b",
    },
  ]);

  // Stage a matching subagent-session record so subagent-traceability rules
  // don't get in the way while we inspect packet.contradictions.
  const rawSub = path.join(runDir, "raw", "subagents", "g4-noop.md");
  fs.writeFileSync(rawSub, "# noop session\n", "utf8");

  runNode(["driver.mjs", "--run-dir", runDir]);
  const packet = JSON.parse(
    fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"),
  );
  assert.equal(
    (packet.contradictions ?? []).length,
    0,
    `G4 observation+proposal pair must not fire a contradiction; got ${JSON.stringify(packet.contradictions)}`,
  );
});

test("G5 contradictions: fired contradictions carry source_refs union for the shared span", (t) => {
  const runDir = freshRunDir("g5-contradiction-refs");
  t.after(() => removeDir(runDir));

  const filePath = path.join(repoRoot, "scripts", "readiness.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));
  const observedAt = new Date().toISOString();
  const ref = {
    source_id: "file:scripts/readiness.mjs:1",
    path: "scripts/readiness.mjs",
    kind: "file",
    hash: expectedHash,
    hash_alg: "fnv1a-64",
    span: "1",
    observed_at: observedAt,
  };
  writeFullEvidence(runDir, [
    {
      id: "ev-objective",
      kind: "objective",
      summary: "G5 test objective",
      source_ids: ["raw:objective.md"],
      observed_at: observedAt,
    },
    {
      id: "ev-g5-change-a",
      kind: "code-change",
      summary: "Patched readiness.mjs:1 to widen the absolute-path check",
      source_ids: [ref.source_id, "raw:objective.md"],
      source_refs: [ref],
      observed_at: observedAt,
      agent_id: "g5-agent-a",
      lane: "g5-lane-a",
    },
    {
      id: "ev-g5-change-b",
      kind: "code-change",
      summary: "Reverted readiness.mjs:1; check only applies to raw kinds",
      source_ids: [ref.source_id, "raw:objective.md"],
      source_refs: [ref],
      observed_at: observedAt,
      agent_id: "g5-agent-b",
      lane: "g5-lane-b",
    },
  ]);

  runNode(["driver.mjs", "--run-dir", runDir]);
  const packet = JSON.parse(
    fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"),
  );
  const contradictions = packet.contradictions ?? [];
  assert.ok(contradictions.length > 0, "G5 two code-change disagreements must fire a contradiction");
  const fired = contradictions[0];
  assert.ok(
    Array.isArray(fired.source_refs) && fired.source_refs.length > 0,
    `G5 contradiction must carry source_refs; got ${JSON.stringify(fired)}`,
  );
  assert.ok(
    fired.source_refs.some(
      (source) => source.source_id === ref.source_id && source.path === ref.path,
    ),
    `G5 source_refs must include the shared file ref; got ${JSON.stringify(fired.source_refs)}`,
  );
});

test("G6 readiness: absolute-path check is active for non-raw packet sources too", (t) => {
  // G6 is a readiness invariant, not a per-run ingest test. Smoke-test the
  // full readiness.mjs pipeline once and assert it still exits 0 — the
  // widened path/absolute check must not regress on the fixture packet.
  const result = runNode(["scripts/readiness.mjs"], { timeout: 300_000 });
  assert.equal(
    result.status,
    0,
    `G6 readiness must pass with widened absolute-path check; stdout=${result.stdout}\nstderr=${result.stderr}`,
  );
});

test("G7 contradictions: severity is high for code-change vs code-change, low for observation vs observation", (t) => {
  const runDirHigh = freshRunDir("g7-high-severity");
  t.after(() => removeDir(runDirHigh));
  const runDirLow = freshRunDir("g7-low-severity");
  t.after(() => removeDir(runDirLow));

  const filePath = path.join(repoRoot, "scripts", "readiness.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));
  const observedAt = new Date().toISOString();
  const ref = {
    source_id: "file:scripts/readiness.mjs:1",
    path: "scripts/readiness.mjs",
    kind: "file",
    hash: expectedHash,
    hash_alg: "fnv1a-64",
    span: "1",
    observed_at: observedAt,
  };
  // HIGH: two code-change records disagree on the same direct span.
  writeFullEvidence(runDirHigh, [
    {
      id: "ev-objective",
      kind: "objective",
      summary: "G7 high test",
      source_ids: ["raw:objective.md"],
      observed_at: observedAt,
    },
    {
      id: "ev-g7-high-a",
      kind: "code-change",
      summary: "Widened assertion coverage across every declared source kind",
      source_ids: [ref.source_id, "raw:objective.md"],
      source_refs: [ref],
      observed_at: observedAt,
      agent_id: "g7-high-a",
      lane: "g7-high-a",
    },
    {
      id: "ev-g7-high-b",
      kind: "code-change",
      summary: "Reverted change and restored raw-only behavior deliberately",
      source_ids: [ref.source_id, "raw:objective.md"],
      source_refs: [ref],
      observed_at: observedAt,
      agent_id: "g7-high-b",
      lane: "g7-high-b",
    },
  ]);
  runNode(["driver.mjs", "--run-dir", runDirHigh]);
  const highPacket = JSON.parse(
    fs.readFileSync(path.join(runDirHigh, "state", "next_pass_packet.json"), "utf8"),
  );
  const highFired = (highPacket.contradictions ?? [])[0];
  assert.ok(highFired, "G7 high-severity scenario must fire a contradiction");
  assert.equal(
    highFired.severity,
    "high",
    `G7 code-change vs code-change must be severity=high; got ${highFired.severity}`,
  );

  // LOW: two observation records disagree.
  writeFullEvidence(runDirLow, [
    {
      id: "ev-objective",
      kind: "objective",
      summary: "G7 low test",
      source_ids: ["raw:objective.md"],
      observed_at: observedAt,
    },
    {
      id: "ev-g7-low-a",
      kind: "observation",
      summary: "Assertion appears active for every declared kind already today",
      source_ids: [ref.source_id, "raw:objective.md"],
      source_refs: [ref],
      observed_at: observedAt,
      agent_id: "g7-low-a",
      lane: "g7-low-a",
    },
    {
      id: "ev-g7-low-b",
      kind: "observation",
      summary: "Inspected file carefully; only raw sources seem guarded here",
      source_ids: [ref.source_id, "raw:objective.md"],
      source_refs: [ref],
      observed_at: observedAt,
      agent_id: "g7-low-b",
      lane: "g7-low-b",
    },
  ]);
  runNode(["driver.mjs", "--run-dir", runDirLow]);
  const lowPacket = JSON.parse(
    fs.readFileSync(path.join(runDirLow, "state", "next_pass_packet.json"), "utf8"),
  );
  const lowFired = (lowPacket.contradictions ?? [])[0];
  assert.ok(lowFired, "G7 low-severity scenario must fire a contradiction");
  assert.equal(
    lowFired.severity,
    "low",
    `G7 observation vs observation must be severity=low; got ${lowFired.severity}`,
  );
});

test("G8 strict-gate: unknown evidence kinds produce a warning, not an error", (t) => {
  const runDir = freshRunDir("g8-unknown-kind");
  t.after(() => removeDir(runDir));

  // Seed everything the gate expects for a passable run, then inject ONE
  // record with an unknown kind.
  const filePath = path.join(repoRoot, "scripts", "readiness.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));
  const observedAt = new Date().toISOString();

  const subagentPath = path.join(runDir, "raw", "subagents", "g8-fixture.md");
  // Include a code-change record (to satisfy R6 direct-ref ratio + R7
  // agent coverage) AND a record with an unknown kind to trigger G8.
  const fixtureRec = {
    id: "ev-g8-anchor",
    kind: "code-change",
    summary: "Anchor record to satisfy direct-ref requirements",
    source_ids: ["file:scripts/readiness.mjs:1", "raw:objective.md"],
    source_refs: [
      {
        source_id: "file:scripts/readiness.mjs:1",
        path: "scripts/readiness.mjs",
        kind: "file",
        hash: expectedHash,
        hash_alg: "fnv1a-64",
        span: "1",
        observed_at: observedAt,
      },
    ],
    observed_at: observedAt,
  };
  const unknownRec = {
    id: "ev-g8-unknown",
    kind: "fictional-kind",
    summary: "Evidence with a kind outside the known-kinds set",
    source_ids: ["raw:objective.md"],
    observed_at: observedAt,
  };
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify(fixtureRec),
      JSON.stringify(unknownRec),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "g8-lane",
    "--agent-id",
    "g8-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `G8 ingest must accept unknown kinds: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  // We only need to prove that strict-gate's output surfaces a warning for
  // the unknown kind. We don't require the full gate to be green — running
  // the gate via `runNode` captures stdout regardless of exit status, and
  // the JSON report always includes the warnings array.
  runNode(["driver.mjs", "--run-dir", runDir]);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  // gate may or may not exit 0 depending on other preconditions — we only
  // care that the G8 warning shows up on the report.
  const report = JSON.parse(gate.stdout);
  assert.ok(Array.isArray(report.warnings), "G8 report must include a warnings array");
  assert.ok(
    report.warnings.some((w) =>
      typeof w === "string" && w.includes("fictional-kind") && w.includes("unknown kind"),
    ),
    `G8 warnings must mention unknown kind 'fictional-kind'; got ${JSON.stringify(report.warnings)}`,
  );
  // And critically: the unknown kind must NOT appear as an error line.
  assert.ok(
    !report.errors.some((e) =>
      typeof e === "string" && e.includes("fictional-kind") && !e.includes("warn"),
    ),
    `G8 unknown kinds must not be errors; got ${JSON.stringify(report.errors)}`,
  );
});

test("G9 ingest: re-ingesting the same raw/subagents file fails with a duplicate-session error", (t) => {
  const runDir = freshRunDir("g9-dup-session");
  t.after(() => removeDir(runDir));

  const subagentPath = path.join(runDir, "raw", "subagents", "g9-fixture.md");
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-g9-anchor",
        kind: "observation",
        summary: "Anchor record so ingest has work to do",
        source_ids: ["raw:objective.md"],
        observed_at: new Date().toISOString(),
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const first = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "g9-lane",
    "--agent-id",
    "g9-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    first.status,
    0,
    `G9 first ingest must exit 0: stdout=${first.stdout}\nstderr=${first.stderr}`,
  );

  const second = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "g9-lane",
    "--agent-id",
    "g9-agent",
    "--from",
    subagentPath,
  ]);
  assert.notEqual(
    second.status,
    0,
    `G9 second ingest on the same raw file must fail: stdout=${second.stdout}\nstderr=${second.stderr}`,
  );
  assert.ok(
    second.stderr.includes("duplicate ingest") || second.stderr.includes("subagent-session record"),
    `G9 failure must mention duplicate-session error; got stderr=${second.stderr}`,
  );
});

test("G10 ingest: absolute source_ref.path on file kinds gets rewritten to repo-relative", (t) => {
  const runDir = freshRunDir("g10-ref-path-normalize");
  t.after(() => removeDir(runDir));

  // Agent declares a file source_ref with an absolute path that resolves
  // inside the repo — after ingest, both source_id and source_refs[*].path
  // must be repo-relative-forward-slash so the compiler verifies against
  // the correct on-disk file regardless of working directory.
  const filePath = path.join(repoRoot, "driver.mjs");
  const expectedHash = fnv1aHashBytes(fs.readFileSync(filePath));
  const absolutePath = filePath.replace(/\\/g, "/");
  const observedAt = new Date().toISOString();

  const subagentPath = path.join(runDir, "raw", "subagents", "g10-fixture.md");
  fs.writeFileSync(
    subagentPath,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-g10-ref",
        kind: "observation",
        summary: "Cites driver.mjs with absolute-path source_ref",
        source_ids: [`file:${absolutePath}:1`, "raw:objective.md"],
        source_refs: [
          {
            source_id: `file:${absolutePath}:1`,
            path: absolutePath,
            kind: "file",
            hash: expectedHash,
            hash_alg: "fnv1a-64",
            span: "1",
            observed_at: observedAt,
          },
        ],
        observed_at: observedAt,
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const ingest = runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    "g10-lane",
    "--agent-id",
    "g10-agent",
    "--from",
    subagentPath,
  ]);
  assert.equal(
    ingest.status,
    0,
    `G10 ingest must exit 0 for absolute-inside-repo refs: stdout=${ingest.stdout}\nstderr=${ingest.stderr}`,
  );

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const fixture = evidence.find((record) => record.id === "ev-g10-ref");
  assert.ok(fixture, "G10 fixture must be present after ingest");
  const ref = (fixture.source_refs ?? []).find(
    (source) => typeof source.source_id === "string" && source.source_id.endsWith("driver.mjs:1"),
  );
  assert.ok(ref, "G10 fixture must keep a driver.mjs source_ref");
  assert.equal(
    ref.source_id,
    "file:driver.mjs:1",
    `G10 source_ref.source_id must be repo-relative; got ${ref.source_id}`,
  );
  assert.equal(
    ref.path,
    "driver.mjs",
    `G10 source_ref.path must be repo-relative; got ${ref.path}`,
  );
});

test("G3 ingest: two concurrent ingests against the same run dir produce a clean JSONL (no interleave)", async (t) => {
  const runDir = freshRunDir("g3-concurrent-ingest");
  t.after(() => removeDir(runDir));

  // Two subagent lanes write separate fenced blocks; each lane calls ingest in
  // parallel. Without G3 advisory locking, the two processes can interleave
  // partial JSON lines into evidence.jsonl. With the O_EXCL lock the appends
  // serialize and every line is parseable.
  const observedAt = new Date().toISOString();
  function writeFixture(laneName, evId) {
    const subagentPath = path.join(runDir, "raw", "subagents", `${laneName}.md`);
    const record = {
      id: evId,
      kind: "observation",
      summary: `concurrent-fixture ${laneName} cites driver.mjs`,
      source_ids: ["file:driver.mjs:1"],
      source_refs: [
        {
          source_id: "file:driver.mjs:1",
          path: "driver.mjs",
          kind: "file",
          hash: "placeholder-will-be-filled",
          hash_alg: "fnv1a-64",
          span: "1",
          observed_at: observedAt,
        },
      ],
      observed_at: observedAt,
    };
    fs.writeFileSync(
      subagentPath,
      ["```receipts-evidence-jsonl", JSON.stringify(record), "```", ""].join("\n"),
      "utf8",
    );
    return subagentPath;
  }

  const laneCount = 5;
  const lanes = Array.from({ length: laneCount }, (_, i) => ({
    name: `concurrent-${i}`,
    evId: `ev-g3-concurrent-${i}`,
    from: writeFixture(`concurrent-${i}`, `ev-g3-concurrent-${i}`),
  }));

  // Fire all ingests in parallel. Collect the spawn promises so Node's test
  // runner waits on the whole fanout.
  function spawnIngest(lane) {
    return new Promise((resolve, reject) => {
      const child = spawn(
        "node",
        [
          "scripts/ingest-subagent.mjs",
          "--run-dir",
          runDir,
          "--lane",
          lane.name,
          "--agent-id",
          `${lane.name}-agent`,
          "--from",
          lane.from,
        ],
        { cwd: repoRoot, shell: process.platform === "win32" },
      );
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (chunk) => {
        stdout += chunk.toString();
      });
      child.stderr.on("data", (chunk) => {
        stderr += chunk.toString();
      });
      child.on("error", reject);
      child.on("close", (code) => resolve({ code, stdout, stderr, lane }));
    });
  }

  const results = await Promise.all(lanes.map((lane) => spawnIngest(lane)));
  for (const result of results) {
    assert.equal(
      result.code,
      0,
      `G3 concurrent ingest for ${result.lane.name} must exit 0: stdout=${result.stdout}\nstderr=${result.stderr}`,
    );
  }

  // Every line in evidence.jsonl must be parseable JSON — if any lane's write
  // interleaved mid-line, JSON.parse will throw on that line.
  const evidenceFile = path.join(runDir, "worker-results", "evidence.jsonl");
  const lines = fs
    .readFileSync(evidenceFile, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  for (const [index, line] of lines.entries()) {
    try {
      JSON.parse(line);
    } catch (err) {
      throw new Error(
        `G3 concurrent ingest corrupted evidence.jsonl line ${index + 1}: ${err.message}\n  line="${line}"`,
      );
    }
  }

  // Every lane's evidence record must appear at least once — ingest cannot
  // drop records under contention.
  const records = lines.map((line) => JSON.parse(line));
  for (const lane of lanes) {
    const found = records.find((record) => record.id === lane.evId);
    assert.ok(
      found,
      `G3 every lane must land its evidence record; missing ${lane.evId}. records=${JSON.stringify(records.map((r) => r.id))}`,
    );
  }

  // Lock sidecar must be cleaned up after the last ingest exits.
  assert.equal(
    fs.existsSync(`${evidenceFile}.lock`),
    false,
    "G3 lock sidecar must be removed after ingest completes",
  );
});

test("G-gate: absolute file: source_id in a hand-crafted packet fails the strict gate", (t) => {
  const runDir = freshRunDir("g-gate-absolute-path-source-id");
  t.after(() => removeDir(runDir));

  // Simulate a packet that slipped past ingest — evidence.jsonl hand-written
  // with a machine-specific absolute source_id. The gate's defense-in-depth
  // check must reject it even though the file exists on this machine.
  const observedAt = new Date().toISOString();
  const absolutePath = path.join(repoRoot, "driver.mjs").replace(/\\/g, "/");
  const driverBytes = fs.readFileSync(path.join(repoRoot, "driver.mjs"));
  // Compute fnv1a-64 locally so the hash is valid — the gate must still
  // reject on path-form grounds, not hash-mismatch grounds.
  function fnv1a(buffer) {
    let hash = 0xcbf29ce484222325n;
    const prime = 0x100000001b3n;
    const mask = 0xffffffffffffffffn;
    for (const byte of buffer) {
      hash ^= BigInt(byte);
      hash = (hash * prime) & mask;
    }
    return hash.toString(16).padStart(16, "0");
  }
  const driverHash = fnv1a(driverBytes);

  const record = {
    id: "ev-gate-abs-path",
    kind: "observation",
    summary: "Evidence leaked an absolute source_id past ingest",
    source_ids: [`file:${absolutePath}:1`, "raw:objective.md"],
    source_refs: [
      {
        source_id: `file:${absolutePath}:1`,
        path: absolutePath,
        kind: "file",
        hash: driverHash,
        hash_alg: "fnv1a-64",
        span: "1",
        observed_at: observedAt,
      },
    ],
    observed_at: observedAt,
  };
  fs.appendFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify(record) + "\n",
    "utf8",
  );

  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir]);
  assert.notEqual(
    gate.status,
    0,
    `G-gate: absolute source_id must fail the gate; stdout=${gate.stdout}\nstderr=${gate.stderr}`,
  );
  const output = `${gate.stdout}\n${gate.stderr}`;
  assert.match(
    output,
    /machine-specific|must be repo-relative/,
    `G-gate failure must mention machine-specific / repo-relative path; got output=${output}`,
  );
});
