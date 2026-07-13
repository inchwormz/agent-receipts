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

function coreRun(runDir, label, exitCode) {
  return spawnSync(
    process.execPath,
    [
      path.join(repoRoot, "bin", "receipts.mjs"),
      "run",
      "--run-dir",
      runDir,
      "--lane",
      "orchestrator",
      "--agent-id",
      "prime",
      ...(label ? ["--label", label] : []),
      "--exe",
      process.execPath,
      "--arg",
      "-e",
      "--arg",
      `process.exit(${exitCode})`,
    ],
    { cwd: repoRoot, encoding: "utf8" },
  );
}

function coreCommand(args, options = {}) {
  return spawnSync(
    process.execPath,
    [path.join(repoRoot, "bin", "receipts.mjs"), ...args],
    { cwd: repoRoot, encoding: "utf8", ...options },
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

test("receipt events stay separate from claims with typed outcomes", (t) => {
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
  assert.equal(fact, undefined, "execution events must never masquerade as claims");
  assert.equal(packet.evidence.some((item) => item.kind === "receipt"), false);
  assert.deepEqual(packet.receipt_events, [
    {
      receipt_id: "rcpt-0001",
      label: "test:demo-pass",
      integrity: "hash_verified",
      outcome: "passed",
      exit_code: 0,
      attempts_for_label: 1,
    },
  ]);
});

test("a failed command cannot improve Evidence Coverage", (t) => {
  const runDir = freshRunDir("failed-coverage");
  t.after(() => removeDir(runDir));

  assert.equal(coreRun(runDir, "test:failed-coverage", 3).status, 3);
  const ingest = ingestLane(
    runDir,
    "coverage-claimer",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-failed-coverage-claim",
        kind: "observation",
        summary: "the failing check passed",
        source_ids: ["test:failed-coverage"],
        observed_at: now(),
      }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(ingest.status, 0, ingest.stderr);
  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.equal(driver.status, 0, driver.stderr);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));

  assert.equal(packet.trusted_facts.some((fact) => fact.id === "fact:ev-failed-coverage-claim"), false);
  assert.deepEqual(packet.evidence_coverage, {
    total_claims: 1,
    verified_claims: 0,
    verifier_backed_claims: 0,
    asserted_claims: 1,
    refuted_claims: 0,
  });
  assert.equal(packet.receipt_events[0].outcome, "failed");
});

test("a passing check becomes stale after its covered subject changes", (t) => {
  const runDir = freshRunDir("subject-freshness");
  t.after(() => removeDir(runDir));
  const projectDir = path.join(runDir, "project");
  fs.mkdirSync(path.join(projectDir, ".receipts"), { recursive: true });
  fs.writeFileSync(path.join(projectDir, "subject.txt"), "version one\n", "utf8");
  fs.writeFileSync(
    path.join(projectDir, ".receipts", "checks.toml"),
    [
      "manifest_version = 1",
      "",
      "[[checks]]",
      'id = "subject-check"',
      'version = "1"',
      `command = [${JSON.stringify(process.execPath)}, "-e", "process.exit(0)"]`,
      'covered_paths = ["subject.txt"]',
      'eligible_claim_kinds = ["observation"]',
      'environment_class = "local"',
      'target_claims = ["ev-bound-subject"]',
      "",
    ].join("\n"),
    "utf8",
  );
  const manifestPath = path.join(runDir, "manifest.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  manifest.repo_root = projectDir;
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");
  fs.appendFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-bound-subject",
      kind: "observation",
      summary: "the subject satisfies its check",
      source_ids: ["raw:objective.md"],
      observed_at: now(),
    }) + "\n",
    "utf8",
  );

  const check = coreCommand(["check", "--run-dir", runDir, "--id", "subject-check"]);
  assert.equal(check.status, 0, `bound check must run: ${check.stdout}\n${check.stderr}`);
  assert.equal(coreCommand(["compile", "--run-dir", runDir]).status, 0);
  let packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  assert.ok(packet.trusted_facts.some((fact) => fact.id === "fact:ev-bound-subject"));
  assert.equal(packet.trust_assessments.find((item) => item.subject_id === "ev-bound-subject").applicability, "current");

  fs.writeFileSync(path.join(projectDir, "subject.txt"), "version two\n", "utf8");
  assert.equal(coreCommand(["compile", "--run-dir", runDir]).status, 0);
  packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  assert.equal(packet.trusted_facts.some((fact) => fact.id === "fact:ev-bound-subject"), false);
  const trust = packet.trust_assessments.find((item) => item.subject_id === "ev-bound-subject");
  assert.equal(trust.applicability, "stale");
  assert.equal(trust.claim_status, "asserted");
});

test("negative controls are expected failures only for the declared signature", (t) => {
  const runDir = freshRunDir("negative-control");
  t.after(() => removeDir(runDir));
  const projectDir = path.join(runDir, "project");
  fs.mkdirSync(path.join(projectDir, ".receipts"), { recursive: true });
  fs.writeFileSync(path.join(projectDir, "subject.txt"), "fixture\n", "utf8");
  const manifestPath = path.join(runDir, "manifest.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  manifest.repo_root = projectDir;
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");
  fs.appendFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-negative-control",
      kind: "observation",
      summary: "the negative control guards this claim",
      source_ids: ["raw:objective.md"],
      observed_at: now(),
    }) + "\n",
    "utf8",
  );

  const writeCheck = (signature, version) => fs.writeFileSync(
    path.join(projectDir, ".receipts", "checks.toml"),
    [
      "manifest_version = 1",
      "",
      "[[checks]]",
      'id = "negative-control"',
      `version = "${version}"`,
      `command = [${JSON.stringify(process.execPath)}, "-e", "process.exit(0)"]`,
      'covered_paths = ["subject.txt"]',
      'eligible_claim_kinds = ["observation"]',
      'environment_class = "local"',
      'target_claims = ["ev-negative-control"]',
      `negative_control_command = [${JSON.stringify(process.execPath)}, "-e", "console.error('BROKEN_FIXTURE'); process.exit(9)"]`,
      `negative_control_expected_signature = "${signature}"`,
      "",
    ].join("\n"),
    "utf8",
  );

  writeCheck("BROKEN_FIXTURE", "1");
  const expected = coreCommand(["check", "--run-dir", runDir, "--id", "negative-control"]);
  assert.equal(expected.status, 0, `${expected.stdout}\n${expected.stderr}`);
  const attemptsPath = path.join(runDir, "checks", "attempts.jsonl");
  let attempts = readJsonl(attemptsPath);
  assert.equal(attempts[0].outcome, "passed");
  assert.equal(attempts[0].negative_control_outcome, "expected_failure");

  assert.equal(coreCommand(["compile", "--run-dir", runDir]).status, 0);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  const controlEvent = packet.receipt_events.find((event) => event.label === "check:negative-control:negative-control");
  assert.equal(controlEvent.outcome, "expected_failure");
  assert.ok(packet.trusted_facts.some((fact) => fact.id === "fact:ev-negative-control"));

  writeCheck("A_DIFFERENT_FAILURE", "1");
  assert.equal(coreCommand(["compile", "--run-dir", runDir]).status, 0);
  const stalePacket = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  assert.equal(stalePacket.trusted_facts.some((fact) => fact.id === "fact:ev-negative-control"), false);
  assert.equal(
    stalePacket.trust_assessments.find((item) => item.subject_id === "ev-negative-control").applicability,
    "stale",
  );
  const wrong = coreCommand(["check", "--run-dir", runDir, "--id", "negative-control"]);
  assert.equal(wrong.status, 1, "wrong failure signature must fail the check");
  attempts = readJsonl(attemptsPath);
  assert.equal(attempts[1].outcome, "failed");
  assert.equal(attempts[1].negative_control_outcome, "failed");
});

test("retry-until-green preserves first result, transitions, and flakiness", (t) => {
  const runDir = freshRunDir("retry-history");
  t.after(() => removeDir(runDir));
  const projectDir = path.join(runDir, "project");
  fs.mkdirSync(path.join(projectDir, ".receipts"), { recursive: true });
  fs.writeFileSync(path.join(projectDir, "subject.txt"), "fixture\n", "utf8");
  const marker = path.join(projectDir, "attempt.marker");
  const command = [
    process.execPath,
    "-e",
    `const fs=require('fs');const p=${JSON.stringify(marker)};if(!fs.existsSync(p)){fs.writeFileSync(p,'1');console.error('FIRST_FAIL');process.exit(7)}`,
  ];
  fs.writeFileSync(
    path.join(projectDir, ".receipts", "checks.toml"),
    [
      "manifest_version = 1",
      "",
      "[[checks]]",
      'id = "retry-check"',
      'version = "1"',
      `command = ${JSON.stringify(command)}`,
      'covered_paths = ["subject.txt"]',
      'eligible_claim_kinds = ["observation"]',
      'environment_class = "local"',
      'target_claims = ["ev-retry"]',
      "",
    ].join("\n"),
    "utf8",
  );
  const manifestPath = path.join(runDir, "manifest.json");
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  manifest.repo_root = projectDir;
  fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");
  fs.appendFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-retry",
      kind: "observation",
      summary: "the retry fixture passes",
      source_ids: ["raw:objective.md"],
      observed_at: now(),
    }) + "\n",
    "utf8",
  );

  assert.equal(coreCommand(["check", "--run-dir", runDir, "--id", "retry-check"]).status, 1);
  assert.equal(coreCommand(["check", "--run-dir", runDir, "--id", "retry-check"]).status, 0);
  assert.equal(coreCommand(["compile", "--run-dir", runDir]).status, 0);
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  assert.deepEqual(packet.check_histories, [
    {
      check_id: "retry-check",
      target_claims: ["ev-retry"],
      first_result: "failed",
      latest_result: "passed",
      attempts: 2,
      attempts_to_green: 2,
      failure_signatures: [readJsonl(path.join(runDir, "checks", "attempts.jsonl"))[0].failure_signature],
      transitions: ["failed->passed"],
      flake_rate: 0.5,
    },
  ]);
  assert.ok(packet.trusted_facts.some((fact) => fact.id === "fact:ev-retry"));
});

test("a check attempt written after compile makes the packet fingerprint stale", (t) => {
  const runDir = freshRunDir("check-fingerprint");
  t.after(() => removeDir(runDir));
  assert.equal(coreCommand(["compile", "--run-dir", runDir]).status, 0);
  fs.mkdirSync(path.join(runDir, "checks"), { recursive: true });
  fs.writeFileSync(path.join(runDir, "checks", "attempts.jsonl"), "{}\n", "utf8");
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
  assert.notEqual(gate.status, 0);
  assert.ok(gate.stdout.includes("next_pass_packet.json is stale"), gate.stdout);
});

test("a copied check-attempt sidecar cannot promote without its verified receipts", (t) => {
  const sourceRun = freshRunDir("copied-check-source");
  const targetRun = freshRunDir("copied-check-target");
  t.after(() => removeDir(sourceRun));
  t.after(() => removeDir(targetRun));
  const projectDir = path.join(sourceRun, "project");
  fs.mkdirSync(path.join(projectDir, ".receipts"), { recursive: true });
  fs.writeFileSync(path.join(projectDir, "subject.txt"), "fixture\n", "utf8");
  fs.writeFileSync(
    path.join(projectDir, ".receipts", "checks.toml"),
    [
      "manifest_version = 1",
      "",
      "[[checks]]",
      'id = "copied-check"',
      'version = "1"',
      `command = [${JSON.stringify(process.execPath)}, "-e", "process.exit(0)"]`,
      'covered_paths = ["subject.txt"]',
      'eligible_claim_kinds = ["observation"]',
      'environment_class = "local"',
      'target_claims = ["ev-copied-check"]',
      "",
    ].join("\n"),
    "utf8",
  );
  for (const runDir of [sourceRun, targetRun]) {
    const manifestPath = path.join(runDir, "manifest.json");
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    manifest.repo_root = projectDir;
    fs.writeFileSync(manifestPath, JSON.stringify(manifest, null, 2) + "\n", "utf8");
    fs.appendFileSync(
      path.join(runDir, "worker-results", "evidence.jsonl"),
      JSON.stringify({
        id: "ev-copied-check",
        kind: "observation",
        summary: "copied attempt must not verify me",
        source_ids: ["raw:objective.md"],
        observed_at: now(),
      }) + "\n",
      "utf8",
    );
  }
  assert.equal(coreCommand(["check", "--run-dir", sourceRun, "--id", "copied-check"]).status, 0);
  fs.mkdirSync(path.join(targetRun, "checks"), { recursive: true });
  fs.copyFileSync(
    path.join(sourceRun, "checks", "attempts.jsonl"),
    path.join(targetRun, "checks", "attempts.jsonl"),
  );

  const compile = coreCommand(["compile", "--run-dir", targetRun]);
  assert.notEqual(compile.status, 0, "sidecar without the referenced receipt must fail closed");
  assert.ok(
    `${compile.stdout}\n${compile.stderr}`.includes("does not reference a verified primary receipt"),
    `${compile.stdout}\n${compile.stderr}`,
  );
});

test("an unbound passing receipt label cannot promote an agent claim", (t) => {
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
  assert.equal(backed, undefined, "caller-selected receipt labels are not semantic bindings");
  assert.equal(
    packet.trust_assessments.find((item) => item.subject_id === "ev-claim-backed").applicability,
    "unbound",
  );
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

test("work receipts remain events and confer nothing on claims citing them", (t) => {
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

  // Work execution is a typed event, not a semantic claim.
  const workFact = packet.trusted_facts.find((f) => f.id === "fact:ev-rcpt-0001");
  assert.equal(workFact, undefined);
  assert.equal(packet.receipt_events.find((event) => event.receipt_id === "rcpt-0001").label, "work:tree");

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

test("a passing receipt label cannot claim semantic verifier coverage", (t) => {
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
  // An arbitrary passing command proves only that it ran; it does not bind
  // this self-authored semantic verifier statement to a subject or claim.
  assert.ok(
    gate.stdout.includes("summary-only verifier findings") && gate.stdout.includes("vf-label-backed"),
    `unbound receipt labels must not create semantic coverage; got ${gate.stdout}`,
  );
});
