// M0 trust-semantics red-team tests (Receipts campaign, plan findings F1-F12).
//
// Each test here encodes an attack or lie that the pre-M0 pipeline ACCEPTED:
// - forged agent diversity beating the coverage floor (F4)
// - a span lie laundered into a "verified" citation (F7)
// - unverified evidence relabeled as trusted fact (F1)
// - gate exemptions triggered by vocabulary in free text (F6)
// - gating without a content fingerprint (F12)
//
// If one of these goes green while its fix is reverted, the gate has a hole.
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const thisFile = fileURLToPath(import.meta.url);
const repoRoot = path.dirname(path.dirname(thisFile));

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

function freshRunDir(name) {
  const stamp = new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
  const runDir = path.join(repoRoot, ".codex", "receipts", `tmp-m0-${stamp}-${process.pid}-${name}`);
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: `obj-m0-${stamp}`,
        run_id: `run-m0-${stamp}`,
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
  fs.writeFileSync(path.join(runDir, "task.md"), `M0 trust semantics: ${name}\n`, "utf8");
  fs.writeFileSync(path.join(runDir, "raw", "objective.md"), `# Objective\n\n${name}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-objective",
      kind: "objective",
      summary: `M0 trust semantics: ${name}`,
      source_ids: ["raw:objective.md"],
      observed_at: new Date().toISOString(),
    }) + "\n",
    "utf8",
  );
  // Real runs always carry the seed synthesis finding (driver createRunDir and
  // `receipts-core init` both write it); an empty findings file is not a real state.
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

function ingest(runDir, lane, agentId, fromPath) {
  return runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    lane,
    "--agent-id",
    agentId,
    "--from",
    fromPath,
  ]);
}

function writeFenced(runDir, name, evidenceRecords, verifierRecords = []) {
  const file = path.join(runDir, "raw", "subagents", `${name}.md`);
  const lines = [];
  if (evidenceRecords.length > 0) {
    lines.push("```receipts-evidence-jsonl");
    for (const record of evidenceRecords) lines.push(JSON.stringify(record));
    lines.push("```", "");
  }
  if (verifierRecords.length > 0) {
    lines.push("```receipts-verifier-jsonl");
    for (const record of verifierRecords) lines.push(JSON.stringify(record));
    lines.push("```", "");
  }
  fs.writeFileSync(file, lines.join("\n"), "utf8");
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

const anchorFile = path.join(repoRoot, "scripts", "readiness.mjs");
const anchorHash = fnv1aHashBytes(fs.readFileSync(anchorFile));
const anchorRef = (span) => ({
  source_id: `file:scripts/readiness.mjs:${span}`,
  path: "scripts/readiness.mjs",
  kind: "file",
  hash: "placeholder-will-be-filled",
  hash_alg: "fnv1a-64",
  span: String(span),
  observed_at: new Date().toISOString(),
});

test("F4: one lane forging three agent identities cannot beat the coverage floor", (t) => {
  const runDir = freshRunDir("forged-coverage");
  t.after(() => removeDir(runDir));

  const observedAt = new Date().toISOString();
  const forged = ["alice", "bob", "carol"].map((fake, index) => ({
    id: `ev-forged-${index}`,
    kind: "observation",
    summary: `Observation ${index} claiming to come from a distinct agent ${fake}`,
    source_ids: ["raw:objective.md"],
    observed_at: observedAt,
    agent_id: fake,
    lane: `lane-${fake}`,
  }));
  const from = writeFenced(runDir, "forged-coverage", forged);
  const result = ingest(runDir, "single-lane", "single-agent", from);
  assert.equal(result.status, 0, `ingest failed: ${result.stderr}`);

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  for (const record of evidence.filter((item) => item.id.startsWith("ev-forged-"))) {
    assert.equal(record.agent_id, "single-agent", "caller stamp must win on every record");
    assert.ok(record.claimed_agent_id, "forged identity must be preserved as claimed_agent_id");
  }

  runNode(["driver.mjs", "--run-dir", runDir]);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "3" },
  });
  assert.notEqual(gate.status, 0, "gate must fail when real coverage is one agent");
  assert.ok(
    gate.stdout.includes("agent-id coverage floor not met"),
    `gate must fail on the coverage floor, not incidentals; got ${gate.stdout}`,
  );
});

test("F7+F1: a span lie is demoted and never becomes a trusted fact, even with verifier support", (t) => {
  const runDir = freshRunDir("span-lie");
  t.after(() => removeDir(runDir));

  const observedAt = new Date().toISOString();
  const liar = {
    id: "ev-span-liar",
    kind: "code-change",
    summary: "Cites a line far beyond the end of the file",
    source_ids: ["file:scripts/readiness.mjs:99999", "raw:objective.md"],
    source_refs: [anchorRef("99999")],
    observed_at: observedAt,
  };
  const backer = {
    id: "vf-span-liar-backer",
    summary: "Passing finding that shares the liar's direct source id",
    status: "passed",
    verifier_score: 1.0,
    source_ids: ["file:scripts/readiness.mjs:99999", "raw:objective.md"],
    source_refs: [anchorRef("99999")],
  };
  const from = writeFenced(runDir, "span-lie", [liar], [backer]);
  const result = ingest(runDir, "span-lane", "span-agent", from);
  assert.equal(result.status, 0, `ingest failed: ${result.stderr}`);

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const ingested = evidence.find((record) => record.id === "ev-span-liar");
  assert.ok(
    Array.isArray(ingested.provenance_warnings) && ingested.provenance_warnings.length > 0,
    "span lie must leave a provenance warning on the record",
  );

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `driver failed: ${driver.stderr}`);
  const packet = JSON.parse(
    fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"),
  );
  const fact = packet.trusted_facts.find((item) => item.id === "fact:ev-span-liar");
  assert.equal(fact, undefined, "demoted record must never reach trusted_facts");
});

test("F1: self-authored verifier evidence cannot promote regardless of verifier_score", (t) => {
  const runDir = freshRunDir("verifier-backing");
  t.after(() => removeDir(runDir));

  const observedAt = new Date().toISOString();
  const backed = {
    id: "ev-backed",
    kind: "code-change",
    summary: "Honest citation of readiness.mjs line 1",
    source_ids: ["file:scripts/readiness.mjs:1", "raw:objective.md"],
    source_refs: [anchorRef("1")],
    observed_at: observedAt,
    confidence: 0.9,
  };
  const unbacked = {
    id: "ev-unbacked",
    kind: "observation",
    summary: "Confident story no verifier ever confirmed",
    source_ids: ["raw:objective.md"],
    observed_at: observedAt,
    confidence: 0.99,
  };
  const backer = {
    id: "vf-backer",
    summary: "Verifier confirms the readiness.mjs citation",
    status: "passed",
    verifier_score: 1.0,
    source_ids: ["file:scripts/readiness.mjs:1", "raw:objective.md"],
    source_refs: [anchorRef("1")],
  };
  const from = writeFenced(runDir, "verifier-backing", [backed, unbacked], [backer]);
  const result = ingest(runDir, "backing-lane", "backing-agent", from);
  assert.equal(result.status, 0, `ingest failed: ${result.stderr}`);

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `driver failed: ${driver.stderr}`);
  const packet = JSON.parse(
    fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"),
  );

  const fact = packet.trusted_facts.find((item) => item.id === "fact:ev-backed");
  assert.equal(fact, undefined, "same-principal verifier must not promote its own claim");
  const trust = packet.trust_assessments.find((item) => item.subject_id === "ev-backed");
  assert.equal(trust.claim_status, "asserted");
  assert.equal(trust.verifier_independent, false);
  const evidence = packet.evidence.find((item) => item.id === "ev-backed");
  assert.ok(Math.abs(evidence.reported_confidence - 0.9) < 1e-3);

  const ghost = packet.trusted_facts.find((item) => item.id === "fact:ev-unbacked");
  assert.equal(ghost, undefined, "self-graded, unbacked evidence must stay out of trusted_facts");
});

test("F6: naming a finding 'subagent' no longer waives the direct-provenance requirement", (t) => {
  const runDir = freshRunDir("vocabulary-gaming");
  t.after(() => removeDir(runDir));

  const sneak = {
    id: "vf-subagent-sneak",
    summary: "subagent synthesis smoke-not-run — every magic word, zero evidence",
    status: "passed",
    verifier_score: 0.95,
    source_ids: ["raw:objective.md"],
  };
  const from = writeFenced(runDir, "vocabulary-gaming", [], [sneak]);
  const result = ingest(runDir, "sneak-lane", "sneak-agent", from);
  assert.equal(result.status, 0, `ingest failed: ${result.stderr}`);

  runNode(["driver.mjs", "--run-dir", runDir]);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(gate.status, 0, "gate must fail on the vocabulary-gamed finding");
  assert.ok(
    gate.stdout.includes("summary-only verifier findings") &&
      gate.stdout.includes("vf-subagent-sneak"),
    `gate must name the sneak finding as summary-only; got ${gate.stdout}`,
  );
});

test("F1: verifier_score zero cannot relax semantic provenance", (t) => {
  const runDir = freshRunDir("zero-score-verifier");
  t.after(() => removeDir(runDir));
  const finding = {
    id: "vf-zero-score",
    summary: "Everything is correct because I said so",
    status: "passed",
    verifier_score: 0.0,
    source_ids: ["raw:objective.md"],
  };
  const from = writeFenced(runDir, "zero-score-verifier", [], [finding]);
  assert.equal(ingest(runDir, "zero-score", "same-worker", from).status, 0);
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(gate.status, 0);
  assert.ok(
    gate.stdout.includes("summary-only verifier findings") && gate.stdout.includes("vf-zero-score"),
    `verifier_score must not change trust or provenance requirements; got ${gate.stdout}`,
  );
});

test("F12: gate refuses to run without a content fingerprint", (t) => {
  const runDir = freshRunDir("missing-fingerprint");
  t.after(() => removeDir(runDir));

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `driver failed: ${driver.stderr}`);
  const fingerprint = path.join(runDir, "state", "input_fingerprint.json");
  assert.ok(fs.existsSync(fingerprint), "compiler must write the fingerprint");
  fs.rmSync(fingerprint);

  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(gate.status, 0, "gate must fail without a fingerprint");
  assert.ok(
    gate.stdout.includes("input_fingerprint.json is missing"),
    `gate must fail closed on the missing fingerprint; got ${gate.stdout}`,
  );
});
