// Loop composites: `absorb` (ingest -> diff -> recompile, one motion per
// lane) and `conclude` (record-synthesis -> gate -> report -> next, one
// motion to end a pass). Dispatcher-level orchestration in bin/receipts.mjs
// — these tests exercise the composite CLI itself, not the underlying
// scripts/binary in isolation (those already have their own suites).
import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const thisFile = fileURLToPath(import.meta.url);
const repoRoot = path.dirname(path.dirname(thisFile));

function freshRunDir(name) {
  const stamp = new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
  const runDir = path.join(repoRoot, ".codex", "receipts", `tmp-lc-${stamp}-${process.pid}-${name}`);
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: `obj-lc-${stamp}`,
        run_id: `run-lc-${stamp}`,
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
  fs.writeFileSync(path.join(runDir, "task.md"), `loop-composite: ${name}\n`, "utf8");
  fs.writeFileSync(path.join(runDir, "raw", "objective.md"), `# Objective\n\n${name}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-objective",
      kind: "objective",
      summary: `loop-composite: ${name}`,
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
  return spawnSync("node", args, { cwd: repoRoot, encoding: "utf8", shell: process.platform === "win32", ...options });
}

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs
    .readFileSync(file, "utf8")
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => JSON.parse(l));
}

function absorb(runDir, lane, agentId, laneFile, extraArgs = []) {
  return runNode([
    "bin/receipts.mjs",
    "absorb",
    "--run-dir",
    runDir,
    "--lane",
    lane,
    "--agent-id",
    agentId,
    "--from",
    laneFile,
    ...extraArgs,
  ]);
}

function conclude(runDir, synthesis, extraArgs = [], options = {}) {
  return runNode(["bin/receipts.mjs", "conclude", "--run-dir", runDir, "--synthesis", synthesis, ...extraArgs], options);
}

function writeProveFixture(name, outcomes, { reportClaimId = "claim:fixture", targetClaimId = "claim:fixture" } = {}) {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), `agent-receipts-prove-${name}-`));
  const checksDir = path.join(fixture, ".receipts");
  fs.mkdirSync(checksDir, { recursive: true });
  fs.writeFileSync(path.join(fixture, "subject.txt"), "fixture subject\n", "utf8");
  const checks = outcomes.map((outcome, index) => [
    "[[checks]]",
    `id = "fixture-${index + 1}"`,
    'version = "1"',
    `command = ["node", "-e", "process.exit(${outcome === "pass" ? 0 : 7})"]`,
    'covered_paths = ["subject.txt"]',
    'eligible_claim_kinds = ["observation"]',
    'environment_class = "test-fixture"',
    `target_claims = ["${targetClaimId}"]`,
    "",
  ].join("\n"));
  fs.writeFileSync(
    path.join(checksDir, "checks.toml"),
    `manifest_version = 1\n\n${checks.join("\n")}`,
    "utf8",
  );
  const report = path.join(fixture, "lane-report.md");
  fs.writeFileSync(
    report,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: reportClaimId,
        kind: "observation",
        summary: "The fixture subject exists",
        source_ids: ["file:subject.txt:1"],
        observed_at: now(),
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  return { fixture, report, runDir: path.join(fixture, "run") };
}

function prove({ runDir, repoRoot: fixtureRoot, reports = [], checks = [], synthesis = "prove fixture synthesis" }, options = {}) {
  const args = [
    "bin/receipts.mjs",
    "prove",
    "--run-dir",
    runDir,
    "--repo-root",
    fixtureRoot,
    "--synthesis",
    synthesis,
  ];
  for (const report of reports) args.push("--report", report);
  for (const check of checks) args.push("--check", check);
  return runNode(args, options);
}

const now = () => new Date().toISOString();

test("absorb happy path: fenced lane record lands in evidence, mints a work:tree receipt, recompiles the packet, prints one ok:true JSON line", (t) => {
  const runDir = freshRunDir("absorb-happy");
  t.after(() => removeDir(runDir));

  // Baseline packet so recompilation can be proven, not assumed.
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  const packetPath = path.join(runDir, "state", "next_pass_packet.json");
  const beforeContent = fs.readFileSync(packetPath, "utf8");
  const beforeMtime = fs.statSync(packetPath).mtimeMs;

  const laneFile = path.join(runDir, "raw", "lane-absorb.md");
  fs.writeFileSync(
    laneFile,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-absorb-claim",
        kind: "observation",
        summary: "Absorb happy-path claim citing a real file in the repo",
        source_ids: ["file:driver.mjs:1"],
        observed_at: now(),
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );

  const result = absorb(runDir, "absorb-lane", "absorb-agent", laneFile);
  assert.equal(result.status, 0, `absorb must succeed: ${result.stderr}`);

  const lines = result.stdout.trim().split(/\r?\n/);
  assert.equal(lines.length, 1, `absorb must print exactly one stdout line; got ${JSON.stringify(result.stdout)}`);
  const parsed = JSON.parse(lines[0]);
  assert.equal(parsed.ok, true);
  assert.equal(parsed.lane, "absorb-lane");
  assert.equal(parsed.compiled, true);
  assert.ok(parsed.ingest && parsed.ingest.ok === true, `ingest field must be step-1's parsed JSON; got ${JSON.stringify(parsed.ingest)}`);
  assert.equal(typeof parsed.work_receipt, "string", "diff ran (no --no-diff) so a work receipt id is expected");

  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  assert.ok(evidence.some((r) => r.id === "ev-absorb-claim"), "lane record must land in evidence.jsonl");

  const receipts = readJsonl(path.join(runDir, "receipts", "receipts.jsonl")).map((line) =>
    line.format_version === "2" ? { ...line.payload, record_hash: line.record_hash } : line,
  );
  assert.ok(
    receipts.some((r) => r.label === "work:tree" && r.id === parsed.work_receipt),
    `a work:tree receipt matching the reported work_receipt id must exist; got ${JSON.stringify(receipts)}`,
  );

  const afterContent = fs.readFileSync(packetPath, "utf8");
  const afterMtime = fs.statSync(packetPath).mtimeMs;
  assert.ok(afterMtime >= beforeMtime, "packet mtime must not regress");
  assert.notEqual(afterContent, beforeContent, "packet content must reflect the newly absorbed lane");
});

test("absorb recompiles packets larger than Node's default spawnSync buffer", (t) => {
  const runDir = freshRunDir("absorb-large-packet");
  t.after(() => removeDir(runDir));

  const evidencePath = path.join(runDir, "worker-results", "evidence.jsonl");
  const padding = "x".repeat(2048);
  const evidence = Array.from({ length: 900 }, (_, index) =>
    JSON.stringify({
      id: `ev-large-packet-${index}`,
      kind: "observation",
      summary: `large packet row ${index} ${padding}`,
      source_ids: ["raw:objective.md"],
      observed_at: now(),
    }),
  ).join("\n");
  fs.appendFileSync(evidencePath, `${evidence}\n`, "utf8");

  const laneFile = path.join(runDir, "raw", "lane-large-packet.md");
  fs.writeFileSync(laneFile, "Large-packet absorb completed.\n", "utf8");

  const result = absorb(runDir, "large-packet-lane", "large-packet-agent", laneFile, ["--no-diff"]);
  const packetPath = path.join(runDir, "state", "next_pass_packet.json");
  assert.ok(fs.existsSync(packetPath), `compiler must write the packet before absorb returns: ${result.stderr}`);
  assert.ok(fs.statSync(packetPath).size > 1024 * 1024, "regression fixture must exceed Node's default spawnSync buffer");
  assert.equal(result.status, 0, `absorb must not fail with ENOBUFS: ${result.stderr}`);
});

test("absorb propagates ingest failure (nonexistent --from file -> nonzero exit)", (t) => {
  const runDir = freshRunDir("absorb-ingest-fail");
  t.after(() => removeDir(runDir));

  const missing = path.join(runDir, "raw", "does-not-exist.md");
  const result = absorb(runDir, "bad-lane", "bad-agent", missing);
  assert.notEqual(result.status, 0, "absorb must fail when ingest fails");
  assert.ok(result.stderr && result.stderr.trim().length > 0, "ingest's stderr must be propagated to absorb's stderr");
});

test("conclude on a green-able run: exit 0, gate-report.json written ok:true, brief printed", (t) => {
  const runDir = freshRunDir("conclude-green");
  t.after(() => removeDir(runDir));

  const laneFile = path.join(runDir, "raw", "lane-green.md");
  fs.writeFileSync(
    laneFile,
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({
        id: "ev-green-claim",
        kind: "observation",
        summary: "Confirmed the driver entrypoint exists as expected",
        source_ids: ["file:driver.mjs:1"],
        observed_at: now(),
      }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  const setup = absorb(runDir, "green-lane", "green-agent", laneFile);
  assert.equal(setup.status, 0, `absorb setup must succeed: ${setup.stderr}`);

  const env = { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" };
  const result = conclude(runDir, "green pass synthesis", [], { env });
  assert.equal(result.status, 0, `conclude must exit 0 on a green run: stdout=${result.stdout}\nstderr=${result.stderr}`);

  const gateReportPath = path.join(runDir, "state", "gate-report.json");
  assert.ok(fs.existsSync(gateReportPath), "gate-report.json must be written");
  const gateReport = JSON.parse(fs.readFileSync(gateReportPath, "utf8"));
  assert.equal(gateReport.ok, true, `gate report must parse with ok:true; got ${JSON.stringify(gateReport)}`);

  assert.ok(result.stdout.includes("RECEIPTS BRIEF"), `stdout must contain the Prime brief; got ${result.stdout}`);
});

test("conclude on a run with an unresolved blocker: exit nonzero, gate-report.json still written ok:false, brief still printed", (t) => {
  const runDir = freshRunDir("conclude-red");
  t.after(() => removeDir(runDir));

  const laneFile = path.join(runDir, "raw", "lane-blocked.md");
  fs.writeFileSync(laneFile, "Tried to reach the target.\n\nBLOCKED could-not-reach-target\n", "utf8");
  const setup = absorb(runDir, "blocked-lane", "blocked-agent", laneFile);
  assert.equal(setup.status, 0, `absorb setup must succeed: ${setup.stderr}`);

  const env = { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" };
  const result = conclude(runDir, "red pass synthesis", [], { env });
  assert.notEqual(result.status, 0, "conclude must exit nonzero when the gate is red (unresolved blocker)");

  const gateReportPath = path.join(runDir, "state", "gate-report.json");
  assert.ok(fs.existsSync(gateReportPath), "gate-report.json must be written even when the gate is red");
  const gateReport = JSON.parse(fs.readFileSync(gateReportPath, "utf8"));
  assert.equal(gateReport.ok, false, `gate report must parse with ok:false; got ${JSON.stringify(gateReport)}`);

  assert.ok(result.stdout.includes("RECEIPTS BRIEF"), `brief must print even when red; got ${result.stdout}`);
});

test("prove happy path: one command initializes, absorbs attributed reports, runs every declared check, concludes, and reports", (t) => {
  const { fixture, report, runDir } = writeProveFixture("green", ["pass", "pass"]);
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));

  const result = prove({
    runDir,
    repoRoot: fixture,
    reports: [`research:agent-one:${report}`],
  }, { env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" } });

  assert.equal(result.status, 0, `prove must succeed: stdout=${result.stdout}\nstderr=${result.stderr}`);
  const firstLine = result.stdout.trim().split(/\r?\n/)[0];
  const summary = JSON.parse(firstLine);
  assert.deepEqual(
    { ok: summary.ok, command: summary.command, reports_absorbed: summary.reports_absorbed, checks_run: summary.checks_run },
    { ok: true, command: "prove", reports_absorbed: 1, checks_run: 2 },
  );
  assert.ok(result.stdout.includes("RECEIPTS BRIEF"), "prove must finish with the existing human brief");
  assert.equal(JSON.parse(fs.readFileSync(path.join(runDir, "manifest.json"), "utf8")).repo_root, fixture);
  assert.equal(readJsonl(path.join(runDir, "checks", "attempts.jsonl")).length, 2, "all declared checks run by default");
  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  assert.equal(packet.evidence_coverage.verified_claims, 1, "the green command must bind at least one report claim to a check");
  assert.equal(JSON.parse(fs.readFileSync(path.join(runDir, "state", "gate-report.json"), "utf8")).ok, true);
  assert.ok(fs.existsSync(path.join(runDir, "state", "report.html")), "prove must render the HTML report");
});

test("prove failed check: returns nonzero after preserving every check attempt and final report", (t) => {
  const { fixture, report, runDir } = writeProveFixture("red", ["fail", "pass"]);
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));

  const result = prove({
    runDir,
    repoRoot: fixture,
    reports: [`research:agent-one:${report}`],
  }, { env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" } });

  assert.notEqual(result.status, 0, "a failed declared check must make prove fail closed");
  const attempts = readJsonl(path.join(runDir, "checks", "attempts.jsonl"));
  assert.deepEqual(attempts.map((attempt) => attempt.outcome), ["failed", "passed"], "prove must not lose later checks after the first red");
  assert.ok(fs.existsSync(path.join(runDir, "state", "gate-report.json")), "the gate still runs after a failed check");
  assert.ok(fs.existsSync(path.join(runDir, "state", "report.html")), "the human report still renders after a failed check");
});

test("prove rejects malformed report attribution before initializing a run", (t) => {
  const { fixture, report, runDir } = writeProveFixture("bad-report", ["pass"]);
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));

  const result = prove({ runDir, repoRoot: fixture, reports: [report] });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /--report.*lane.*agent.*path/i);
  assert.equal(fs.existsSync(runDir), false, "invalid attribution must fail before creating run state");
});

test("prove refuses a false green when checks pass but zero report claims are bound", (t) => {
  const { fixture, report, runDir } = writeProveFixture(
    "unbound",
    ["pass"],
    { reportClaimId: "claim:not-covered", targetClaimId: "claim:fixture" },
  );
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));

  const result = prove({
    runDir,
    repoRoot: fixture,
    reports: [`research:agent-one:${report}`],
  }, { env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" } });

  assert.notEqual(result.status, 0, "a passing command with zero bound facts must fail closed");
  assert.match(result.stdout, /"bound_claims":0/);
  assert.ok(fs.existsSync(path.join(runDir, "state", "report.html")), "the diagnostic report must still be available");
});

test("prove fails closed when the repository has no check manifest", (t) => {
  const { fixture, report, runDir } = writeProveFixture("no-manifest", ["pass"]);
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  fs.rmSync(path.join(fixture, ".receipts", "checks.toml"));

  const result = prove({
    runDir,
    repoRoot: fixture,
    reports: [`research:agent-one:${report}`],
  });

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /no \.receipts[\\/]checks\.toml/i);
});

test("prove fails closed when the final HTML report cannot be rendered", (t) => {
  const { fixture, report, runDir } = writeProveFixture("report-failure", ["pass"]);
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const initialized = runNode(["bin/receipts.mjs", "init", runDir, "--repo-root", fixture]);
  assert.equal(initialized.status, 0, initialized.stderr);
  fs.mkdirSync(path.join(runDir, "state", "report.html"), { recursive: true });

  const result = prove({
    runDir,
    repoRoot: fixture,
    reports: [`research:agent-one:${report}`],
  }, { env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" } });

  assert.notEqual(result.status, 0, "prove must not pass when its human report failed to render");
  assert.match(result.stdout, /"report_ok":false/);
});
