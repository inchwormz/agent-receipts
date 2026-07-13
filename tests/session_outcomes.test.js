import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cli = path.join(repoRoot, "bin", "receipts.mjs");

function command(args, env = {}) {
  return spawnSync(process.execPath, [cli, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

function readJsonl(file) {
  return fs.readFileSync(file, "utf8").trim().split(/\r?\n/).map(JSON.parse);
}

test("captured exact session plus bound check reaches a signed independent outcome", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-session-outcome-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const project = path.join(fixture, "project");
  const runDir = path.join(fixture, "run");
  fs.mkdirSync(path.join(project, ".receipts"), { recursive: true });
  fs.writeFileSync(path.join(project, "subject.txt"), "verified subject\n", "utf8");
  fs.writeFileSync(path.join(project, "review.txt"), "Independent human reviewed hidden expectations.\n", "utf8");
  fs.writeFileSync(
    path.join(project, ".receipts", "checks.toml"),
    [
      "manifest_version = 1",
      "",
      "[[checks]]",
      'id = "outcome-check"',
      'version = "1"',
      `command = [${JSON.stringify(process.execPath)}, "-e", "process.exit(0)"]`,
      'covered_paths = ["subject.txt"]',
      'eligible_claim_kinds = ["observation"]',
      'environment_class = "local"',
      'target_claims = ["ev-outcome"]',
      "",
    ].join("\n"),
    "utf8",
  );
  assert.equal(command(["init", runDir, "--repo-root", project]).status, 0);

  fs.mkdirSync(path.join(runDir, "session"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "session", "generic.json"),
    JSON.stringify({
      provider: "openai",
      requested_model: "gpt-5",
      resolved_model_snapshot: "gpt-5-2026-07-01",
      provider_session_id: "sess-fixture-1",
      agent_name: "codex",
      agent_version: "1.2.3",
      scaffold_name: "receipts-fixture",
      scaffold_version: "1",
      tool_configuration: { shell: "powershell", network: false },
      reasoning_setting: { effort: "high" },
    }) + "\n",
    "utf8",
  );
  const capture = command(["session", "capture", "--run-dir", runDir, "--adapter", "generic"]);
  assert.equal(capture.status, 0, `${capture.stdout}\n${capture.stderr}`);
  const sessions = readJsonl(path.join(runDir, "sessions", "sessions.jsonl"));
  assert.equal(sessions[0].record_kind, "session_capture");
  assert.equal(sessions[0].payload.resolution_status, "resolved");
  assert.equal(sessions[0].payload.resolved_model_snapshot, "gpt-5-2026-07-01");
  assert.match(sessions[0].signature, /^[0-9a-f]{128}$/);

  const lane = path.join(fixture, "lane.md");
  fs.writeFileSync(
    lane,
    '```receipts-evidence-jsonl\n{"id":"ev-outcome","kind":"observation","summary":"subject meets the task","source_ids":["file:subject.txt:1"]}\n```\n',
    "utf8",
  );
  assert.equal(command(["ingest", "--run-dir", runDir, "--lane", "worker", "--agent-id", "codex", "--from", lane]).status, 0);
  assert.equal(command(["check", "--run-dir", runDir, "--id", "outcome-check"]).status, 0);
  assert.equal(command(["compile", "--run-dir", runDir]).status, 0);

  const adjudicate = command([
    "adjudicate",
    "--run-dir",
    runDir,
    "--result",
    "success",
    "--grade",
    "signed-human-review",
    "--cite",
    "file:review.txt",
  ]);
  assert.equal(adjudicate.status, 0, `${adjudicate.stdout}\n${adjudicate.stderr}`);
  const outcomes = readJsonl(path.join(runDir, "outcomes", "outcomes.jsonl"));
  assert.equal(outcomes[0].record_kind, "independent_outcome");
  assert.equal(outcomes[0].payload.result, "success");
  assert.equal(outcomes[0].payload.training_eligibility, "included");
  assert.equal(outcomes[0].payload.model.resolved_model_snapshot, "gpt-5-2026-07-01");
  assert.ok(outcomes[0].payload.claim_ids.includes("ev-outcome"));
  assert.match(outcomes[0].signature, /^[0-9a-f]{128}$/);

  const unknown = command([
    "adjudicate",
    "--run-dir",
    runDir,
    "--result",
    "unknown",
    "--grade",
    "signed-human-review",
    "--cite",
    "file:review.txt",
  ]);
  assert.equal(unknown.status, 0, `${unknown.stdout}\n${unknown.stderr}`);
  const withUnknown = readJsonl(path.join(runDir, "outcomes", "outcomes.jsonl"));
  assert.equal(withUnknown[1].payload.result, "unknown");
  assert.equal(withUnknown[1].payload.training_eligibility, "excluded");

  const outcomeJournal = path.join(runDir, "outcomes", "outcomes.jsonl");
  withUnknown[0].payload.result = "failure";
  fs.writeFileSync(outcomeJournal, `${withUnknown.map(JSON.stringify).join("\n")}\n`, "utf8");
  const afterTamper = command([
    "adjudicate",
    "--run-dir",
    runDir,
    "--result",
    "success",
    "--grade",
    "signed-human-review",
    "--cite",
    "file:review.txt",
  ]);
  assert.notEqual(afterTamper.status, 0, "a changed signed outcome must fail closed");
  assert.match(afterTamper.stderr, /signed independent_outcome record hash mismatch at entry 1/);
});

test("an unresolved mutable model alias is captured but excluded from model-specific outcomes", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-unresolved-model-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const runDir = path.join(fixture, "run");
  assert.equal(command(["init", runDir, "--repo-root", repoRoot]).status, 0);
  fs.mkdirSync(path.join(runDir, "session"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "session", "generic.json"),
    JSON.stringify({ provider: "openai", requested_model: "gpt-5", agent_name: "codex" }) + "\n",
  );
  const capture = command(["session", "capture", "--run-dir", runDir, "--adapter", "generic"]);
  assert.equal(capture.status, 0, `${capture.stdout}\n${capture.stderr}`);
  const payload = readJsonl(path.join(runDir, "sessions", "sessions.jsonl"))[0].payload;
  assert.equal(payload.resolution_status, "unresolved");
  assert.equal(payload.resolved_model_snapshot, null);
  assert.equal(payload.model_specific_eligible, false);

  fs.writeFileSync(
    path.join(runDir, "session", "generic.json"),
    JSON.stringify({
      provider: "openai",
      requested_model: "gpt-5",
      resolved_model_snapshot: "gpt-5-2026-07-01",
      agent_name: "codex",
    }) + "\n",
  );
  const incomplete = command(["session", "capture", "--run-dir", runDir, "--adapter", "generic"]);
  assert.equal(incomplete.status, 0, `${incomplete.stdout}\n${incomplete.stderr}`);
  const incompletePayload = readJsonl(path.join(runDir, "sessions", "sessions.jsonl"))[1].payload;
  assert.equal(incompletePayload.resolution_status, "resolved");
  assert.equal(incompletePayload.resolved_model_snapshot, "gpt-5-2026-07-01");
  assert.equal(incompletePayload.model_specific_eligible, false, "missing agent version is not an exact model-agent identity");
});
