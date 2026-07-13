// M1/M2 acceptance: execution receipts and the attestation ladder.
//
// The product promise under test: valuable receipts WITHOUT agent
// cooperation. The orchestrator runs commands through `receipts run`; the
// journal is hash-chained; compile turns receipts into attested facts,
// upgrades agent claims whose cited labels actually passed, and mechanically
// REFUTES passed claims whose cited labels actually failed. Agents cannot
// mint, fake, or edit receipts.
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const thisFile = fileURLToPath(import.meta.url);
const repoRoot = path.dirname(path.dirname(thisFile));
const compilerDir = path.join(repoRoot, "receipts-compiler");

function freshRunDir(name) {
  const stamp = new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
  const runDir = path.join(repoRoot, ".codex", "receipts", `tmp-rcpt-${stamp}-${process.pid}-${name}`);
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: `obj-rc-${stamp}`,
        run_id: `run-rc-${stamp}`,
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
  fs.writeFileSync(path.join(runDir, "task.md"), `receipts: ${name}\n`, "utf8");
  fs.writeFileSync(path.join(runDir, "raw", "objective.md"), `# Objective\n\n${name}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-objective",
      kind: "objective",
      summary: `receipts: ${name}`,
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

// Invoke the dev binary the same way the driver does (source checkout).
function coreRun(runDir, label, exitCode) {
  return spawnSync(
    "cargo",
    [
      "run",
      "--quiet",
      "--bin",
      "receipts-core",
      "--",
      "run",
      "--run-dir",
      runDir,
      "--lane",
      "orchestrator",
      "--agent-id",
      "prime",
      ...(label ? ["--label", label] : []),
      "--",
      "node",
      "-e",
      // Quote-free on purpose: this arg transits shell:true concatenation on
      // Windows, so quotes/semicolons would be mangled before reaching node.
      `process.exit(${exitCode})`,
    ],
    { cwd: compilerDir, encoding: "utf8", shell: process.platform === "win32" },
  );
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

function ingestLane(runDir, lane, content) {
  const file = path.join(runDir, "raw", "subagents", `${lane}.md`);
  fs.writeFileSync(file, content, "utf8");
  return runNode([
    "scripts/ingest-subagent.mjs",
    "--run-dir",
    runDir,
    "--lane",
    lane,
    "--agent-id",
    `${lane}-agent`,
    "--from",
    file,
  ]);
}

const now = () => new Date().toISOString();

test("receipts run mints chained receipts and propagates the child exit code", (t) => {
  const runDir = freshRunDir("mint");
  t.after(() => removeDir(runDir));

  const pass = coreRun(runDir, "test:demo-pass", 0);
  assert.equal(pass.status, 0, `passing child must yield exit 0: ${pass.stderr}`);
  const fail = coreRun(runDir, "test:demo-fail", 3);
  assert.equal(fail.status, 3, `child exit code must propagate; got ${fail.status}`);

  const journal = readJsonl(path.join(runDir, "receipts", "receipts.jsonl"));
  assert.equal(journal.length, 2, "journal must contain both receipts");
  assert.equal(journal[0].id, "rcpt-0001");
  assert.equal(journal[0].exit_code, 0);
  assert.equal(journal[1].exit_code, 3);
  assert.equal(journal[1].prev_record_hash, journal[0].record_hash, "chain must link");
  assert.ok(journal[0].stdout_hash.match(/^[0-9a-f]{16}$/));
  const artifact = path.join(runDir, "receipts", "artifacts", `${journal[0].stdout_hash}.txt`);
  assert.ok(fs.existsSync(artifact), "stdout artifact must be stored content-addressed");
});

test("receipts compile into attested facts with zero agent cooperation", (t) => {
  const runDir = freshRunDir("attested");
  t.after(() => removeDir(runDir));

  coreRun(runDir, "test:demo-pass", 0);
  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `compile must succeed: ${driver.stderr}`);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));

  assert.ok(
    packet.sources.some((s) => s.source_id === "receipt:rcpt-0001" && s.kind === "receipt" && s.hash_basis === "content"),
    "packet must carry the receipt as a content-hashed source",
  );
  const fact = packet.trusted_facts.find((f) => f.id === "fact:ev-rcpt-0001");
  assert.ok(fact, "receipt must become a trusted fact");
  assert.equal(fact.attestation, "attested", "receipt facts carry the attested tier");
});

test("an agent claim citing a label whose receipt PASSED becomes an attested fact", (t) => {
  const runDir = freshRunDir("label-upgrade");
  t.after(() => removeDir(runDir));

  coreRun(runDir, "test:demo-pass", 0);
  const ingest = ingestLane(
    runDir,
    "claimer",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-claim-backed", kind: "observation", summary: "the demo check passes", source_ids: ["test:demo-pass"], observed_at: now() }),
      JSON.stringify({ id: "ev-claim-unbacked", kind: "observation", summary: "some other check passes", source_ids: ["test:never-ran"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest.status, 0, `ingest failed: ${ingest.stderr}`);

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `compile failed: ${driver.stderr}`);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));

  const backed = packet.trusted_facts.find((f) => f.id === "fact:ev-claim-backed");
  assert.ok(backed, "label-backed claim must promote");
  assert.equal(backed.attestation, "attested");
  assert.equal(
    packet.trusted_facts.find((f) => f.id === "fact:ev-claim-unbacked"),
    undefined,
    "claim citing a label with no receipt must stay unpromoted",
  );
});

test("a passed claim whose label FAILED on execution is refuted and turns the gate red", (t) => {
  const runDir = freshRunDir("refuted");
  t.after(() => removeDir(runDir));

  coreRun(runDir, "test:demo-fail", 3);
  const ingest = ingestLane(
    runDir,
    "liar",
    [
      "```receipts-verifier-jsonl",
      JSON.stringify({ id: "vf-liar-suite", summary: "ran the demo-fail suite, everything green", status: "passed", verifier_score: 1.0, source_ids: ["test:demo-fail"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest.status, 0, `ingest failed: ${ingest.stderr}`);

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `compile failed: ${driver.stderr}`);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  const refutation = packet.contradictions.find((c) => String(c.id).startsWith("con:receipt:vf-liar-suite"));
  assert.ok(refutation, `compile must emit a receipt refutation; got ${JSON.stringify(packet.contradictions)}`);
  assert.equal(refutation.severity, "high");

  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(gate.status, 0, "gate must go red on a receipt refutation");
  assert.ok(
    gate.stdout.includes("refuted by execution receipt"),
    `gate must name the refutation; got ${gate.stdout}`,
  );
});

test("agents cannot impersonate receipts or cite unminted ones", (t) => {
  const runDir = freshRunDir("impersonation");
  t.after(() => removeDir(runDir));

  const ingest = ingestLane(
    runDir,
    "faker",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-fake-receipt", kind: "receipt", summary: "totally ran this myself", source_ids: ["receipt:rcpt-9999"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest.status, 0, `ingest must survive the fake: ${ingest.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const rec = evidence.find((r) => r.id === "ev-fake-receipt");
  assert.equal(rec.kind, "observation", "impersonated receipt kind must demote to observation");
  assert.ok(
    rec.source_ids.some((id) => id.startsWith("log:unminted-receipt-")),
    `unminted receipt citation must downgrade; got ${JSON.stringify(rec.source_ids)}`,
  );
  assert.ok((rec.provenance_warnings ?? []).some((w) => w.startsWith("receipt-impersonation")));
});

test("work receipts attest tree state but confer NOTHING on claims citing them", (t) => {
  const runDir = freshRunDir("work-receipt");
  t.after(() => removeDir(runDir));

  // Mint a work receipt via the real subcommand (repo_root is this repo).
  const diff = spawnSync(
    "cargo",
    ["run", "--quiet", "--bin", "receipts-core", "--", "diff", "--run-dir", runDir, "--note", "phase-1-fixture"],
    { cwd: compilerDir, encoding: "utf8", shell: process.platform === "win32" },
  );
  assert.equal(diff.status, 0, `receipts diff must succeed: ${diff.stderr}`);
  const journal = readJsonl(path.join(runDir, "receipts", "receipts.jsonl"));
  assert.equal(journal[0].label, "work:tree", "work receipts carry the constant label");

  // A lane tries to ride the work receipt: cites the label AND the id.
  const ingest = ingestLane(
    runDir,
    "rider",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-rider", kind: "code-change", summary: "I did all of that tree work", source_ids: ["work:tree", "receipt:rcpt-0001"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest.status, 0, `rider ingest must survive: ${ingest.stderr}`);

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, `compile failed: ${driver.stderr}`);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));

  // The work receipt itself IS an attested fact about the tree...
  const workFact = packet.trusted_facts.find((f) => f.id === "fact:ev-rcpt-0001");
  assert.ok(workFact, "work receipt must compile to an attested tree-state fact");
  assert.equal(workFact.attestation, "attested");
  assert.ok(workFact.statement.startsWith("tree delta:"), workFact.statement);

  // ...but the rider gains nothing: its claim stays out of trusted_facts.
  assert.equal(
    packet.trusted_facts.find((f) => f.id === "fact:ev-rider"),
    undefined,
    "citing a work receipt/label must never attest a claim",
  );
  const rider = packet.evidence.find((e) => e.id === "ev-rider");
  assert.ok(
    rider.source_ids.some((id) => id.startsWith("log:")),
    `the work:tree citation must have been downgraded to log:*; got ${JSON.stringify(rider.source_ids)}`,
  );

  // Gate: the rider's citations are not content anchors either.
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.ok(
    gate.stdout.includes("summary-only evidence") && gate.stdout.includes("ev-rider"),
    `rider must be flagged summary-only despite citing the work receipt; got ${gate.stdout}`,
  );
});

test("a tampered journal breaks the chain and fails compile", (t) => {
  const runDir = freshRunDir("tamper");
  t.after(() => removeDir(runDir));

  coreRun(runDir, "test:demo-pass", 0);
  const journalPath = path.join(runDir, "receipts", "receipts.jsonl");
  fs.writeFileSync(journalPath, fs.readFileSync(journalPath, "utf8").replace('"exit_code":0', '"exit_code":1'), "utf8");

  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.notEqual(driver.status, 0, "compile must fail on a tampered journal");
  assert.ok(
    `${driver.stdout}\n${driver.stderr}`.includes("chain broken"),
    `compile must name the broken chain; got ${driver.stderr}`,
  );
});

test("a passing receipt upgrades a label-only finding past the summary-only check", (t) => {
  const runDir = freshRunDir("label-finding");
  t.after(() => removeDir(runDir));

  coreRun(runDir, "test:demo-pass", 0);
  const ingest = ingestLane(
    runDir,
    "verifier",
    [
      "```receipts-verifier-jsonl",
      JSON.stringify({ id: "vf-label-backed", summary: "demo-pass suite is green", status: "passed", verifier_score: 1.0, source_ids: ["test:demo-pass"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest.status, 0, `ingest failed: ${ingest.stderr}`);
  runNode(["driver.mjs", "--run-dir", runDir]);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  // The gate may fail for other reasons (pending synthesis etc.) - what must
  // NOT appear is a summary-only complaint about the receipt-backed finding.
  assert.ok(
    !gate.stdout.includes("summary-only verifier findings lack direct file/command provenance: vf-label-backed") &&
      !(gate.stdout.includes("summary-only verifier findings") && gate.stdout.includes("vf-label-backed")),
    `receipt-backed label finding must not be summary-only; got ${gate.stdout}`,
  );
});
