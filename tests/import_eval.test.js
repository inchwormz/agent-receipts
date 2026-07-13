import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import test from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const cli = path.join(repoRoot, "bin", "receipts.mjs");

test("pinned exact task-level eval imports with signed provenance and fractional weight", (t) => {
  const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "receipts-import-eval-"));
  t.after(() => fs.rmSync(fixture, { recursive: true, force: true }));
  const source = path.join(fixture, "eval.json");
  const out = path.join(fixture, "imports");
  fs.writeFileSync(
    source,
    JSON.stringify({
      format_version: "1",
      data_kind: "task-results",
      source_url: "https://example.invalid/eval/v1",
      retrieval_date: "2026-07-14",
      methodology_version: "method-v1",
      harness_version: "harness-v2",
      sample_size: 2,
      attribution: "Fixture Evaluation Authors",
      license: "CC-BY-4.0",
      records: [
        { task_id: "task-1", result: "success", provider: "openai", model_snapshot: "gpt-5-2026-07-01", agent_name: "codex", agent_version: "1.2.3", task_family: "bugfix", repository_id: "repo-a", language: "rust" },
        { task_id: "task-2", result: "failure", provider: "openai", model_snapshot: "gpt-5-2026-07-01", agent_name: "codex", agent_version: "1.2.3", task_family: "bugfix", repository_id: "repo-b", language: "rust" },
      ],
    }) + "\n",
  );
  const result = spawnSync(process.execPath, [cli, "import-eval", "--from", source, "--out", out], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
  const report = JSON.parse(result.stdout);
  assert.equal(report.prior_weight, 0.25);
  assert.equal(report.sample_size, 2);
  assert.match(report.dataset_hash, /^[0-9a-f]{64}$/);
  assert.ok(fs.existsSync(path.join(out, `${report.dataset_hash}.json`)));
  const receipt = JSON.parse(fs.readFileSync(path.join(out, `${report.dataset_hash}.receipt.json`), "utf8"));
  assert.equal(receipt.dataset_hash, report.dataset_hash);
  assert.match(receipt.signature, /^[0-9a-f]{128}$/);

  const duplicate = spawnSync(process.execPath, [cli, "import-eval", "--from", source, "--out", out], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.notEqual(duplicate.status, 0, "the same dataset must not be imported twice");
});
