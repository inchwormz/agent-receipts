import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cli = path.join(repoRoot, "bin", "receipts.mjs");

function receipts(args) {
  return spawnSync(process.execPath, [cli, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
  });
}

test("public projection is deterministic and omits private run material", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-public-projection-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const runDir = path.join(fixture, "run");
  const firstOut = path.join(fixture, "public-1.json");
  const secondOut = path.join(fixture, "public-2.json");
  const init = receipts(["init", runDir, "--repo-root", repoRoot]);
  assert.equal(init.status, 0, init.stderr);
  fs.writeFileSync(
    path.join(runDir, "task.md"),
    "PRIVATE_PROMPT_MARKER sk-private-token C:\\Users\\johnr\\private https://private.invalid/repo\n",
    "utf8",
  );
  const run = receipts([
    "run",
    "--run-dir",
    runDir,
    "--label",
    "test:private-material",
    "--exe",
    process.execPath,
    "--arg",
    "-e",
    "--arg",
    "process.stdout.write('PRIVATE_STDOUT_MARKER')",
  ]);
  assert.equal(run.status, 0, run.stderr);

  const first = receipts(["project-public", "--run-dir", runDir, "--out", firstOut]);
  assert.equal(first.status, 0, `${first.stdout}\n${first.stderr}`);
  const second = receipts(["project-public", "--run-dir", runDir, "--out", secondOut]);
  assert.equal(second.status, 0, `${second.stdout}\n${second.stderr}`);
  const firstBytes = fs.readFileSync(firstOut);
  const secondBytes = fs.readFileSync(secondOut);
  assert.deepEqual(firstBytes, secondBytes, "the same on-disk run must project byte-for-byte identically");

  const text = firstBytes.toString("utf8");
  for (const forbidden of [
    "PRIVATE_PROMPT_MARKER",
    "PRIVATE_STDOUT_MARKER",
    "sk-private-token",
    "C:\\\\Users",
    "private.invalid",
    "repo_root",
    "objective",
    "stdout",
    "stderr",
    "source_ids",
    "cmd",
  ]) {
    assert.equal(text.includes(forbidden), false, `public projection leaked ${forbidden}`);
  }
  const projection = JSON.parse(text);
  assert.equal(projection.projection_version, "1.0.0");
  assert.equal(projection.schema_version, "2.0.0");
  assert.ok(projection.evidence_coverage);
  assert.ok(projection.receipts);
  assert.ok(projection.claims);
});
