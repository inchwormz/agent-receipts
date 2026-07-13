#!/usr/bin/env node
// One-shot helper: strip source_refs that point at <run-dir>/state/* from
// ingested evidence and verifier records. Self-state references drift on every
// recompile (because the compiler regenerates those files), so evidence that
// cites them becomes hash-invalid immediately after the next pass.
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

function isSelfState(refOrId) {
  const str = String(refOrId ?? "").replace(/\\/g, "/");
  return /\.codex\/(?:mythos|receipts)\/runs\/[^/]+\/state\//.test(str);
}

function scrubFile(file, fallbackSourceId) {
  if (!fs.existsSync(file)) return { touched: 0, total: 0 };
  const lines = fs.readFileSync(file, "utf8").split(/\r?\n/).filter(Boolean);
  let touched = 0;
  const fixed = lines.map((line) => {
    const obj = JSON.parse(line);
    let changed = false;
    if (Array.isArray(obj.source_refs)) {
      const before = obj.source_refs.length;
      obj.source_refs = obj.source_refs.filter((r) => !isSelfState(r.path) && !isSelfState(r.source_id));
      if (obj.source_refs.length !== before) changed = true;
    }
    if (Array.isArray(obj.source_ids)) {
      const before = obj.source_ids.length;
      obj.source_ids = obj.source_ids.filter((id) => !isSelfState(id));
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
  const evResult = scrubFile(path.join(runDir, "worker-results/evidence.jsonl"), "raw:subagents/unknown.md");
  const vfResult = scrubFile(path.join(runDir, "verifier-results/findings.jsonl"), "raw:subagents/unknown.md");
  process.stdout.write(JSON.stringify({
    ok: true,
    evidence_touched: evResult.touched,
    evidence_total: evResult.total,
    findings_touched: vfResult.touched,
    findings_total: vfResult.total,
  }, null, 2) + "\n");
}

main();
