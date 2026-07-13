// Forgiving-ingest tests (post-mortem of the first live NTM run, 2026-07-12,
// run mythos-zero-visual-44: agents wrote shorthand, bare fences, pretty JSON,
// prose reports - every deviation was a hard rejection and the orchestrator
// abandoned the system). Each test reproduces an OBSERVED failure class as a
// synthetic fixture and asserts the lane now survives with repairs recorded.
// Trust semantics must not weaken: repairs are logged, unverifiable citations
// are downgraded to non-provenance log: ids, prose lanes yield demoted
// `unstructured` records that never count as facts or coverage.
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const thisFile = fileURLToPath(import.meta.url);
const repoRoot = path.dirname(path.dirname(thisFile));

function freshRunDir(name) {
  const stamp = new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
  const runDir = path.join(repoRoot, ".codex", "receipts", `tmp-forgive-${stamp}-${process.pid}-${name}`);
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: `obj-fg-${stamp}`,
        run_id: `run-fg-${stamp}`,
        branch_id: "main",
        pass_id: "pass-0002",
        created_at: new Date().toISOString(),
        repo_root: repoRoot,
      },
      null,
      2,
    ) + "\n",
    "utf8",
  );
  fs.writeFileSync(path.join(runDir, "task.md"), `forgiving ingest: ${name}\n`, "utf8");
  fs.writeFileSync(path.join(runDir, "raw", "objective.md"), `# Objective\n\n${name}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-objective",
      kind: "objective",
      summary: `forgiving ingest: ${name}`,
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
      finding_kind: "synthesis",
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
  return spawnSync("node", args, {
    cwd: repoRoot,
    encoding: "utf8",
    shell: process.platform === "win32",
    ...options,
  });
}

function ingest(runDir, lane, fromPath) {
  return runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    lane,
    "--agent-id",
    `${lane}-agent`,
    "--from",
    fromPath,
  ]);
}

function writeLane(runDir, name, content) {
  const file = path.join(runDir, "raw", "subagents", `${name}.md`);
  fs.writeFileSync(file, content, "utf8");
  return file;
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

const now = () => new Date().toISOString();

test("bare ``` fence with JSONL records is classified by content and ingested", (t) => {
  const runDir = freshRunDir("bare-fence");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "bare-fence",
    [
      "# My report",
      "",
      "Some prose the agent wrote.",
      "",
      "```",
      JSON.stringify({ id: "ev-bare-1", kind: "observation", summary: "found the thing", source_ids: ["file:driver.mjs:1"], observed_at: now() }),
      JSON.stringify({ id: "vf-bare-1", summary: "checked the thing", status: "passed", verifier_score: 1.0, source_ids: ["file:driver.mjs:1"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "bare-fence", from);
  assert.equal(result.status, 0, `bare-fence ingest must succeed: ${result.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const findings = readJsonl(path.join(runDir, "verifier-results", "findings.jsonl"));
  assert.ok(evidence.some((r) => r.id === "ev-bare-1"), "evidence record must land from bare fence");
  assert.ok(findings.some((r) => r.id === "vf-bare-1"), "verifier record must be routed by shape from the same bare fence");
});

test("pretty-printed JSON with single quotes, unquoted keys, and trailing commas gets repaired", (t) => {
  const runDir = freshRunDir("repair-json");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "repair-json",
    [
      "```receipts-evidence-jsonl",
      "{",
      "  id: 'ev-pretty-1',",
      "  kind: 'observation',",
      "  summary: 'multi-line pretty record with sloppy quoting',",
      "  source_ids: ['file:driver.mjs:1'],",
      `  observed_at: '${now()}',`,
      "}",
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "repair-json", from);
  assert.equal(result.status, 0, `repair ingest must succeed: ${result.stderr}\n${result.stdout}`);
  const report = JSON.parse(result.stdout);
  assert.ok(report.repairs > 0, "repairs must be counted in the report");
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  assert.ok(evidence.some((r) => r.id === "ev-pretty-1"), "repaired record must land");
});

test("whole-block JSON array of records is accepted", (t) => {
  const runDir = freshRunDir("array-block");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "array-block",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify([
        { id: "ev-arr-1", kind: "observation", summary: "first", source_ids: ["file:driver.mjs:1"], observed_at: now() },
        { id: "ev-arr-2", kind: "observation", summary: "second", source_ids: ["file:package.json:1"], observed_at: now() },
      ], null, 2),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "array-block", from);
  assert.equal(result.status, 0, `array ingest must succeed: ${result.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  assert.ok(evidence.some((r) => r.id === "ev-arr-1") && evidence.some((r) => r.id === "ev-arr-2"));
});

test("field aliases and bare path:line citations normalize (text->summary, type->kind, sources->source_ids)", (t) => {
  const runDir = freshRunDir("aliases");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "aliases",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ type: "observation", text: "shorthand record with a bare citation", sources: "driver.mjs:12", timestamp: now() }),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "aliases", from);
  assert.equal(result.status, 0, `alias ingest must succeed: ${result.stderr}\n${result.stdout}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => typeof r.summary === "string" && r.summary.includes("shorthand record"));
  assert.ok(rec, "aliased record must land");
  assert.equal(rec.kind, "observation");
  assert.ok(rec.id, "missing id must be defaulted");
  assert.ok(
    rec.source_ids.includes("file:driver.mjs:12"),
    `bare citation must coerce to file:driver.mjs:12; got ${JSON.stringify(rec.source_ids)}`,
  );
});

test("prose-only lane survives as a demoted unstructured record instead of a hard failure", (t) => {
  const runDir = freshRunDir("prose-only");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "prose-only",
    [
      "# Deep investigation report",
      "",
      "I checked the freeze pipeline thoroughly and found the animation issue.",
      "The root cause is the terminal-state drive not running before freeze.",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "prose-only", from);
  assert.equal(result.status, 0, `prose-only ingest must exit 0 now: ${result.stderr}`);
  const report = JSON.parse(result.stdout);
  assert.equal(report.unstructured, true, "report must flag the lane as unstructured");
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => r.kind === "unstructured");
  assert.ok(rec, "unstructured record must be synthesized");
  assert.ok(rec.summary.includes("animation issue") || rec.summary.includes("prose"), "summary must carry a prose excerpt");
  assert.ok(
    (rec.provenance_warnings ?? []).length > 0,
    "unstructured record must be demoted via provenance warning",
  );

  // It must never become a fact and never count toward coverage.
  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `driver must compile: ${driver.stderr}`);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  assert.ok(
    !packet.trusted_facts.some((f) => f.id.includes("unstructured")),
    "unstructured records must never promote to trusted_facts",
  );
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(gate.status, 0, "one prose-only lane must not satisfy even a floor of 1");
  assert.ok(
    gate.stdout.includes("agent-id coverage floor not met"),
    `unstructured lanes must not count toward coverage; got ${gate.stdout}`,
  );
});

test("zero-burden: claims are harvested from natural prose with no protocol at all", (t) => {
  const runDir = freshRunDir("prose-harvest");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "prose-harvest",
    [
      "# Investigation report",
      "",
      "Dug into the freeze pipeline this morning. Findings:",
      "",
      "- The escape helper in scripts/strict-gate.mjs:164 hashes with fnv1a, same constants as the Rust side.",
      "- driver.mjs:93 duplicates that hash function again, third copy in the repo.",
      "- I think the real fix belongs in receipts-compiler/src/compiler/run_dir.rs somewhere around the source dedupe.",
      "",
      "No blockers, just flagging the duplication.",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "prose-harvest", from);
  assert.equal(result.status, 0, `prose harvest ingest must succeed: ${result.stderr}`);
  const report = JSON.parse(result.stdout);
  assert.ok(report.harvested >= 3, `expected >=3 harvested claims, got ${report.harvested}`);
  assert.equal(report.unstructured, false, "harvested lanes are structured, not unstructured");

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const harvested = evidence.filter((r) => r.rationale === "harvested-from-prose");
  assert.ok(harvested.length >= 3, "harvested records must be marked with their origin");
  const gateClaim = harvested.find((r) => r.summary.includes("escape helper"));
  assert.ok(gateClaim, "the strict-gate claim must be harvested");
  assert.ok(
    gateClaim.source_ids.includes("file:scripts/strict-gate.mjs:164"),
    `bare path citation must coerce to a canonical file id; got ${JSON.stringify(gateClaim.source_ids)}`,
  );
  assert.ok(
    !(gateClaim.provenance_warnings ?? []).length,
    `honest existing-path citations must NOT demote the record; got ${JSON.stringify(gateClaim.provenance_warnings)}`,
  );
});

test("zero-burden: a harvested prose claim can still become a trusted fact once verified", (t) => {
  const runDir = freshRunDir("prose-promote");
  t.after(() => removeDir(runDir));

  // Lane 1: pure prose (no protocol followed at all).
  const proseLane = writeLane(
    runDir,
    "prose-worker",
    "Looked at the dispatcher. The run passthrough lives in bin/receipts.mjs:70 and forwards args verbatim.\n",
  );
  assert.equal(ingest(runDir, "prose-worker", proseLane).status, 0);

  // Lane 2: a verifier that backs the same citation.
  const verifierLane = writeLane(
    runDir,
    "verify-worker",
    [
      "```receipts-verifier-jsonl",
      JSON.stringify({ id: "vf-backs-prose", summary: "confirmed the dispatcher forwards args verbatim", status: "passed", verifier_score: 1.0, source_ids: ["file:bin/receipts.mjs:70"], observed_at: new Date().toISOString() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest(runDir, "verify-worker", verifierLane).status, 0);

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `compile failed: ${driver.stderr}`);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  const promoted = packet.trusted_facts.find(
    (f) => f.statement.includes("dispatcher") || (f.source_ids ?? []).includes("file:bin/receipts.mjs:70"),
  );
  assert.ok(
    promoted,
    `a verifier-backed harvested claim must promote to trusted_facts; facts: ${JSON.stringify(packet.trusted_facts.map((f) => f.id))}`,
  );
  assert.equal(promoted.attestation, "verifier");
});

test("nonexistent file citation is downgraded to log:unverifiable, lane survives, record never a fact", (t) => {
  const runDir = freshRunDir("ghost-citation");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "ghost-citation",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-ghost", kind: "code-change", summary: "cites a file that does not exist", source_ids: ["file:no/such/file.rs:10"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "ghost-citation", from);
  assert.equal(result.status, 0, `ghost-citation ingest must survive: ${result.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => r.id === "ev-ghost");
  assert.ok(rec, "record must survive with downgraded citation");
  assert.ok(
    rec.source_ids.some((id) => id.startsWith("log:unverifiable-")),
    `citation must downgrade to log:unverifiable-*; got ${JSON.stringify(rec.source_ids)}`,
  );
  assert.ok((rec.provenance_warnings ?? []).some((w) => w.startsWith("unverifiable-citation:")));

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `compile must succeed with downgraded citations: ${driver.stderr}`);
});

test("a demoted record (unresolvable citation) does not double-jeopardy the gate as summary-only", (t) => {
  const runDir = freshRunDir("no-double-jeopardy");
  t.after(() => removeDir(runDir));

  // Harvested-style record whose only concrete citation cannot resolve: it
  // must be demoted (warning, never a fact) but must NOT red the gate as
  // summary-only - demotion is the penalty.
  const from = writeLane(
    runDir,
    "demoted-lane",
    "Checked the flow end to end. The conclusion writer lands in state/ghost-file.json:1 as expected.\n",
  );
  const result = ingest(runDir, "demoted-lane", from);
  assert.equal(result.status, 0, `ingest failed: ${result.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => r.rationale === "harvested-from-prose");
  assert.ok(rec, "claim must be harvested");
  assert.ok((rec.provenance_warnings ?? []).length > 0, "unresolvable citation must demote");

  runNode(["driver.mjs", "--run-dir", runDir]);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.ok(
    !gate.stdout.includes("summary-only evidence"),
    `demoted records must not be red-carded as summary-only; got ${gate.stdout}`,
  );
});

test("free-text citations become log: ids and the packet still compiles", (t) => {
  const runDir = freshRunDir("freeform-citation");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "freeform-citation",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-freeform", kind: "observation", summary: "cites prose not a path", source_ids: ["manual inspection of the board"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "freeform-citation", from);
  assert.equal(result.status, 0, `freeform ingest must survive: ${result.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => r.id === "ev-freeform");
  assert.ok(rec.source_ids.some((id) => id.startsWith("log:")), `freeform citation must become log:*; got ${JSON.stringify(rec.source_ids)}`);
  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `packet must compile with log: citations: ${driver.stderr}`);
});

test("agent-declared source_refs are discarded and resynthesized (the field test's biggest killer)", (t) => {
  const runDir = freshRunDir("refs-discarded");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "refs-discarded",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-refs",
        kind: "observation",
        summary: "carries hand-written refs with a bogus hash and unsupported alg",
        source_ids: ["file:driver.mjs:1"],
        source_refs: [{ source_id: "file:driver.mjs:1", path: "driver.mjs", kind: "file", hash: "NOT-A-REAL-HASH", hash_alg: "sha256", span: "1", observed_at: "not-a-date" }],
        observed_at: now(),
      }),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "refs-discarded", from);
  assert.equal(result.status, 0, `hand-written refs must never kill a lane: ${result.stderr}\n${result.stdout}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => r.id === "ev-refs");
  const ref = (rec.source_refs ?? []).find((s) => s.source_id === "file:driver.mjs:1");
  assert.ok(ref, "ref must be resynthesized by ingest");
  assert.equal(ref.hash_alg, "fnv1a-64", "resynthesized ref must carry the canonical alg");
  assert.match(ref.hash, /^[0-9a-f]{16}$/, "resynthesized hash must be computed from disk, not copied from the agent");
});

test("unknown verifier status and stringly score are parked, not rejected", (t) => {
  const runDir = freshRunDir("weird-verifier");
  t.after(() => removeDir(runDir));

  const from = writeLane(
    runDir,
    "weird-verifier",
    [
      "```receipts-verifier-jsonl",
      JSON.stringify({ id: "vf-weird", summary: "status vocabulary drift", status: "verified", verifier_score: "1.0", source_ids: ["file:driver.mjs:1"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  const result = ingest(runDir, "weird-verifier", from);
  assert.equal(result.status, 0, `weird verifier must survive: ${result.stderr}\n${result.stdout}`);
  const findings = readJsonl(path.join(runDir, "verifier-results", "findings.jsonl"));
  const rec = findings.find((r) => r.id === "vf-weird");
  assert.equal(rec.status, "proposed", "unrecognized status must park as proposed (non-passing, gate-visible)");
  assert.equal(rec.verifier_score, 1.0, "stringly score must coerce to number");
});
