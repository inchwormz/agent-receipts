#!/usr/bin/env node
// One-shot helper: strip source_refs that point at self-mutating files inside
// a run. These references become hash-invalid as soon as the next pass appends
// a receipt, resolution, evidence record, finding, or regenerated state file.
//
// Usage: node scripts/scrub-self-state.mjs --run-dir <path>
import fs from "node:fs";
import path from "node:path";

function fail(msg) { process.stderr.write(msg + "\n"); process.exit(1); }

function parseArgs(argv) {
  const runFlag = argv.indexOf("--run-dir");
  if (runFlag === -1 || !argv[runFlag + 1]) fail("usage: scrub-self-state.mjs --run-dir <path>");
  return { runDir: path.resolve(argv[runFlag + 1]) };
}

const SELF_MUTATING_RUN_FILES = new Set([
  "receipts/receipts.jsonl",
  "decisions/resolutions.jsonl",
  "worker-results/evidence.jsonl",
  "verifier-results/findings.jsonl",
]);

function sourcePath(refOrId) {
  let value = String(refOrId ?? "").replace(/\\/g, "/");
  if (value.startsWith("file:")) value = value.slice("file:".length).replace(/:\d+(?:-\d+)?$/, "");
  return value;
}

function selfMutatingRunPath(refOrId, runDir) {
  const value = sourcePath(refOrId);
  if (!value) return null;
  let relative;
  if (path.isAbsolute(value)) {
    relative = path.relative(runDir, path.resolve(value));
    if (relative === "" || relative === ".." || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) return null;
  } else {
    relative = value;
  }
  const normalized = relative.replace(/\\/g, "/").replace(/^\.\//, "");
  return normalized.startsWith("state/") || SELF_MUTATING_RUN_FILES.has(normalized)
    ? normalized
    : null;
}

function scrubFile(file, fallbackSourceId, runDir) {
  if (!fs.existsSync(file)) return { touched: 0, total: 0 };
  const lines = fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean);
  let touched = 0;
  const fixed = lines.map((line) => {
    const obj = JSON.parse(line);
    let changed = false;
    if (Array.isArray(obj.source_refs)) {
      const before = obj.source_refs.length;
      obj.source_refs = obj.source_refs.filter((r) => !selfMutatingRunPath(r.path, runDir) && !selfMutatingRunPath(r.source_id, runDir));
      if (obj.source_refs.length !== before) changed = true;
    }
    if (Array.isArray(obj.source_ids)) {
      const before = obj.source_ids.length;
      obj.source_ids = obj.source_ids.filter((id) => !selfMutatingRunPath(id, runDir));
      if (obj.source_ids.length !== before) changed = true;
      if (obj.source_ids.length === 0 && fallbackSourceId) obj.source_ids = [fallbackSourceId];
    }
    if (changed) touched++;
    return JSON.stringify(obj);
  }).join("\n") + "\n";
  fs.writeFileSync(file, fixed);
  return { touched, total: lines.length };
}

function main() {
  const { runDir } = parseArgs(process.argv.slice(2));
  if (!fs.existsSync(runDir)) fail(`run dir does not exist: ${runDir}`);
  const evResult = scrubFile(path.join(runDir, "worker-results/evidence.jsonl"), "raw:subagents/unknown.md", runDir);
  const vfResult = scrubFile(path.join(runDir, "verifier-results/findings.jsonl"), "raw:subagents/unknown.md", runDir);
  process.stdout.write(JSON.stringify({
    ok: true,
    evidence_touched: evResult.touched,
    evidence_total: evResult.total,
    findings_touched: vfResult.touched,
    findings_total: vfResult.total,
  }, null, 2) + "\n");
}

main();
