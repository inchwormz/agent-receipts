import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
const root = path.resolve(process.argv[2] ?? "");
if (!process.argv[2]) throw new Error("usage: node tests/trainer_smoke_fixture.mjs <output-dir>");
const runs = path.join(root, "runs");
const imports = path.join(root, "imports");
const source = path.join(root, "eval.json");
const dataset = path.join(root, "calibration.dataset.json");
fs.mkdirSync(runs, { recursive: true });
const records = Array.from({ length: 60 }, (_, index) => ({
  task_id: `smoke-task-${index}`,
  result: index % 3 === 0 ? "failure" : "success",
  provider: "openai",
  model_snapshot: `smoke-model-${index % 5}`,
  agent_name: "codex",
  agent_version: `smoke-agent-${index % 5}`,
  task_family: `family-${index % 3}`,
  repository_id: `repo-${index % 10}`,
  language: index % 2 === 0 ? "rust" : "typescript",
}));
fs.mkdirSync(root, { recursive: true });
fs.writeFileSync(source, `${JSON.stringify({
  format_version: "1",
  data_kind: "task-results",
  source_url: "https://example.invalid/receipts-trainer-smoke/v1",
  retrieval_date: "2026-07-14",
  methodology_version: "smoke-v1",
  harness_version: "smoke-harness-v1",
  sample_size: records.length,
  attribution: "Agent Receipts smoke fixture",
  license: "CC-BY-4.0",
  records,
})}\n`);
const cli = path.join(repoRoot, "bin", "receipts.mjs");
for (const args of [
  ["import-eval", "--from", source, "--out", imports],
  ["calibration", "dataset", "--runs", runs, "--imports", imports, "--out", dataset],
]) {
  const result = spawnSync(process.execPath, [cli, ...args], { cwd: repoRoot, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`${result.stdout}\n${result.stderr}`);
}
process.stdout.write(`${dataset}\n`);
