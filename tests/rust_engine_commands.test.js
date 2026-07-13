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

test("Rust engine owns ingest, compile, synthesis, and categorical gate", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-rust-engine-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const runDir = path.join(fixture, "run");
  assert.equal(command(["init", runDir, "--repo-root", repoRoot]).status, 0);
  const lane = path.join(fixture, "lane.md");
  fs.writeFileSync(
    lane,
    [
      "```receipts-evidence-jsonl",
      "{id: 'ev-rust-ingest', type: 'observation', text: 'README exists in the checked project', sources: ['file:README.md:1'], agent_id: 'forged-agent',}",
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  const ingest = command([
    "ingest",
    "--run-dir",
    runDir,
    "--lane",
    "rust-lane",
    "--agent-id",
    "authenticated-caller",
    "--from",
    lane,
  ]);
  assert.equal(ingest.status, 0, `${ingest.stdout}\n${ingest.stderr}`);
  const evidence = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl"));
  const record = evidence.find((item) => item.id === "ev-rust-ingest");
  assert.equal(record.agent_id, "authenticated-caller");
  assert.equal(record.claimed_agent_id, "forged-agent");
  assert.ok(record.source_refs.some((source) => source.kind === "file" && source.hash_basis === "content"));
  assert.ok(evidence.some((item) => item.kind === "subagent-session"));

  const compile = command(["compile", "--run-dir", runDir]);
  assert.equal(compile.status, 0, `${compile.stdout}\n${compile.stderr}`);
  const synthesize = command([
    "synthesize",
    "--run-dir",
    runDir,
    "--summary",
    "Reviewed the current packet and retained its source-backed observation.",
  ]);
  assert.equal(synthesize.status, 0, `${synthesize.stdout}\n${synthesize.stderr}`);
  const gate = command(["gate", "--run-dir", runDir], {
    RECEIPTS_MIN_AGENT_COVERAGE: "1",
  });
  assert.equal(gate.status, 0, `${gate.stdout}\n${gate.stderr}`);
  assert.equal(JSON.parse(gate.stdout).ok, true);
});
