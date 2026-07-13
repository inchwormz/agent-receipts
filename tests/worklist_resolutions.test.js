// Phase 2 red-team: the derived worklist and the resolutions journal.
//
// The adversarial design review's finding 3: blocking categories without a
// clearing mechanism deadlock the gate by construction (contradictions and
// blockers are recomputed from append-only inputs forever). These tests prove
// the full circuit: blocking item -> gate red -> `receipts-core resolve` -> recompile
// -> gate green; and that advisory items never red the gate.
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
  const runDir = path.join(repoRoot, ".codex", "receipts", `tmp-wl-${stamp}-${process.pid}-${name}`);
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify(
      {
        objective_id: `obj-wl-${stamp}`,
        run_id: `run-wl-${stamp}`,
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
  fs.writeFileSync(path.join(runDir, "task.md"), `worklist: ${name}\n`, "utf8");
  fs.writeFileSync(path.join(runDir, "raw", "objective.md"), `# Objective\n\n${name}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-objective",
      kind: "objective",
      summary: `worklist: ${name}`,
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

function coreBin(args) {
  return spawnSync("cargo", ["run", "--quiet", "--bin", "receipts", "--", ...args], {
    cwd: compilerDir,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
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

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs
    .readFileSync(file, "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

function gate(runDir) {
  return runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], {
    env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" },
  });
}

const now = () => new Date().toISOString();

test("blocked lane -> gate red -> receipts resolve -> gate green (no deadlock)", (t) => {
  const runDir = freshRunDir("unblock-circuit");
  t.after(() => removeDir(runDir));

  const ingest = ingestLane(runDir, "stuck-lane", "Tried to reach the flag console.\n\nBLOCKED flag-console-unreachable\n");
  assert.equal(ingest.status, 0, `blocked-lane ingest must succeed: ${ingest.stderr}`);
  const blocker = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl")).find(
    (r) => r.kind === "blocker",
  );
  assert.ok(blocker, "blocker record must exist");

  // Compile + synthesis (the normal loop), then the gate must be RED on the
  // unresolved blocking item.
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  assert.equal(
    runNode(["driver.mjs", "--run-dir", runDir, "--record-synthesis", "adjudication test synthesis"]).status,
    0,
  );
  const red = gate(runDir);
  assert.notEqual(red.status, 0, "gate must be red while the blocker is unresolved");
  assert.ok(
    red.stdout.includes("unresolved blocking worklist item [unblock]"),
    `gate must name the unblock item with guidance; got ${red.stdout}`,
  );
  assert.ok(red.stdout.includes("receipts resolve"), "gate error must teach the clearing command");

  // Prime adjudicates, recompiles - green.
  const resolve = coreBin(["resolve", "--run-dir", runDir, "--target", blocker.id, "--reason", "lane-restarted-with-credentials"]);
  assert.equal(resolve.status, 0, `resolve must succeed: ${resolve.stderr}`);
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  const green = gate(runDir);
  assert.equal(green.status, 0, `gate must be green after resolution: ${green.stdout}`);

  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  const item = packet.candidate_actions.find((a) => a.category === "unblock");
  assert.equal(item.resolved, true, "worklist item must be marked resolved");
  assert.match(item.resolution_id, /^res-\d{4}$/);
});

test("high-severity contradiction blocks until adjudicated; advisory verify-claim items never red the gate", (t) => {
  const runDir = freshRunDir("adjudicate-circuit");
  t.after(() => removeDir(runDir));

  // Two lanes make DIVERGENT code-change claims about the same span (fires a
  // high-severity auto-contradiction), plus one uncontradicted code-change
  // claim citing an unbacked label (advisory verify-claim material).
  const refA = { source_id: "file:driver.mjs:1", path: "driver.mjs", kind: "file", hash: "placeholder-will-be-filled", hash_alg: "fnv1a-64", span: "1", observed_at: now() };
  const a = ingestLane(
    runDir,
    "lane-a",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-a-change", kind: "code-change", summary: "Rewrote the dispatcher header to add strict mode enforcement", source_ids: ["file:driver.mjs:1"], source_refs: [refA], observed_at: now() }),
      JSON.stringify({ id: "ev-a-solo", kind: "code-change", summary: "Standalone change nobody contests", source_ids: ["file:package.json:1", "test:never-verified"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(a.status, 0, a.stderr);
  const b = ingestLane(
    runDir,
    "lane-b",
    [
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-b-change", kind: "code-change", summary: "Reverted every dispatcher edit; the header is untouched original", source_ids: ["file:driver.mjs:1"], source_refs: [refA], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(b.status, 0, b.stderr);

  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  assert.equal(
    runNode(["driver.mjs", "--run-dir", runDir, "--record-synthesis", "contradiction adjudication test"]).status,
    0,
  );

  const packet1 = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  const adjudicate = packet1.candidate_actions.find((a2) => a2.category === "adjudicate");
  assert.ok(adjudicate, "adjudicate item must exist for the high-severity contradiction");
  assert.equal(adjudicate.blocking, true, "code-change vs code-change contradiction is blocking");
  const verify = packet1.candidate_actions.filter((a2) => a2.category === "verify-claim");
  assert.ok(verify.length >= 1, "verify-claim items must exist for unbacked load-bearing claims");
  assert.ok(verify.every((a2) => a2.blocking === false), "verify-claim is advisory");
  const withArgv = verify.find((a2) => (a2.suggested_argv ?? []).length > 0);
  assert.ok(withArgv, "label-citing claim must get a suggested argv");
  assert.deepEqual(
    withArgv.suggested_argv,
    ["receipts", "run", "--run-dir", "<run-dir>", "--label", "test:never-verified", "--"],
    "argv must be engine tokens with a placeholder run dir",
  );

  const red = gate(runDir);
  assert.notEqual(red.status, 0, "gate must be red on the unresolved adjudication");
  assert.ok(red.stdout.includes("unresolved blocking worklist item [adjudicate]"), red.stdout);
  assert.ok(
    !red.stdout.includes("[verify-claim]"),
    "advisory items must never appear as gate errors",
  );

  const target = adjudicate.decision_dependency_ids[0];
  const resolve = coreBin(["resolve", "--run-dir", runDir, "--target", target, "--reason", "lane-b-is-correct-verified-by-prime", "--cite", "file:driver.mjs:1"]);
  assert.equal(resolve.status, 0, resolve.stderr);
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  const green = gate(runDir);
  assert.equal(green.status, 0, `gate must be green after adjudication: ${green.stdout}`);
});

test("a tampered resolutions journal fails compile (adjudications are custody-tracked)", (t) => {
  const runDir = freshRunDir("res-tamper");
  t.after(() => removeDir(runDir));

  const ingest = ingestLane(runDir, "stuck", "BLOCKED cannot-proceed\n");
  assert.equal(ingest.status, 0);
  const blocker = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl")).find((r) => r.kind === "blocker");
  assert.equal(coreBin(["resolve", "--run-dir", runDir, "--target", blocker.id, "--reason", "fine-actually"]).status, 0);

  const journal = path.join(runDir, "decisions", "resolutions.jsonl");
  fs.writeFileSync(journal, fs.readFileSync(journal, "utf8").replace("fine-actually", "totally-different-reason"), "utf8");
  const driver = runNode(["driver.mjs", "--run-dir", runDir]);
  assert.notEqual(driver.status, 0, "compile must fail on tampered resolutions");
  assert.ok(`${driver.stdout}\n${driver.stderr}`.includes("chain broken"), driver.stderr);
});
