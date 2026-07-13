// Phase 3: drill-down spans, lane digests, and the `receipts-core next` brief.
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
  const runDir = path.join(repoRoot, ".codex", "receipts", `tmp-brief-${stamp}-${process.pid}-${name}`);
  fs.mkdirSync(path.join(runDir, "raw", "subagents"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "worker-results"), { recursive: true });
  fs.mkdirSync(path.join(runDir, "verifier-results"), { recursive: true });
  fs.writeFileSync(
    path.join(runDir, "manifest.json"),
    JSON.stringify({
      objective_id: `obj-br-${stamp}`,
      run_id: `run-br-${stamp}`,
      branch_id: "main",
      pass_id: "pass-0002",
      created_at: new Date().toISOString(),
      repo_root: repoRoot,
    }, null, 2) + "\n",
    "utf8",
  );
  fs.writeFileSync(path.join(runDir, "task.md"), `brief: ${name}\n`, "utf8");
  fs.writeFileSync(path.join(runDir, "raw", "objective.md"), `# Objective\n\n${name}\n`, "utf8");
  fs.writeFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({ id: "ev-objective", kind: "objective", summary: `brief: ${name}`, source_ids: ["raw:objective.md"], observed_at: new Date().toISOString() }) + "\n",
    "utf8",
  );
  fs.writeFileSync(
    path.join(runDir, "verifier-results", "findings.jsonl"),
    JSON.stringify({ id: "vf-codex-synthesis-pending", summary: "Codex synthesis has not consumed this packet yet", status: "pending", verifier_score: 0.0, source_ids: ["raw:objective.md"], finding_kind: "synthesis" }) + "\n",
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

function coreBin(args) {
  return spawnSync("cargo", ["run", "--quiet", "--bin", "receipts-core", "--", ...args], {
    cwd: compilerDir,
    encoding: "utf8",
    shell: process.platform === "win32",
  });
}

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8").split(/\r?\n/).map((l) => l.trim()).filter(Boolean).map((l) => JSON.parse(l));
}

const now = () => new Date().toISOString();

test("drill-down: records carry span-suffixed raw citations computed from the FINAL quarantined bytes", (t) => {
  const runDir = freshRunDir("spans");
  t.after(() => removeDir(runDir));

  // Write the lane file OUTSIDE raw/subagents so ingest quarantines it with
  // the 5-line session header - the span must reflect post-header lines
  // (finding 9's off-by-header bug).
  const outside = path.join(runDir, "raw", "lane-input.md");
  fs.writeFileSync(
    outside,
    [
      "Report prose before the block.",
      "",
      "```receipts-evidence-jsonl",
      JSON.stringify({ id: "ev-span-probe", kind: "observation", summary: "span probe claim", source_ids: ["file:driver.mjs:1"], observed_at: now() }),
      "```",
      "",
    ].join("\n"),
    "utf8",
  );
  const ingest = runNode([
    "scripts/ingest-subagent.mjs", "--run-dir", runDir, "--lane", "span-lane", "--agent-id", "span-agent", "--from", outside,
  ]);
  assert.equal(ingest.status, 0, `ingest failed: ${ingest.stderr}`);

  const record = readJsonl(path.join(runDir, "worker-results", "evidence.jsonl")).find((r) => r.id === "ev-span-probe");
  const spanId = record.source_ids.find((id) => /^raw:subagents\/.+:\d+-\d+$/.test(id));
  assert.ok(spanId, `record must carry a span-suffixed raw citation; got ${JSON.stringify(record.source_ids)}`);
  const [, spanText] = spanId.match(/:(\d+-\d+)$/);
  const [start] = spanText.split("-").map(Number);
  assert.ok(start > 5, `span must account for the quarantine header (start ${start} must be > 5)`);
  const ref = record.source_refs.find((s) => s.source_id === spanId);
  assert.ok(ref, "span citation must have a matching hash-bearing source_ref");
  assert.equal(ref.span, spanText);

  // The quarantined file's cited lines must actually contain the record.
  const quarantined = fs.readFileSync(path.join(runDir, "raw", "subagents", path.basename(record.source_refs.find((s) => s.source_id === spanId).path)), "utf8").split(/\r?\n/);
  assert.ok(quarantined[start - 1].includes("ev-span-probe"), `line ${start} must contain the record`);

  // Compile + gate: spans validate clean.
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], { env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" } });
  assert.ok(!gate.stdout.includes("outside file line range"), `raw spans must validate; got ${gate.stdout}`);
});

test("gate rejects raw spans pointing outside the quarantined file", (t) => {
  const runDir = freshRunDir("bad-span");
  t.after(() => removeDir(runDir));

  const rawFile = path.join(runDir, "raw", "subagents", "fake.md");
  fs.writeFileSync(rawFile, "# fake session\nshort file\n", "utf8");
  const hash = (() => {
    let h = 0xcbf29ce484222325n;
    for (const b of fs.readFileSync(rawFile)) { h ^= BigInt(b); h = (h * 0x100000001b3n) & 0xffffffffffffffffn; }
    return h.toString(16).padStart(16, "0");
  })();
  const observedAt = now();
  fs.appendFileSync(
    path.join(runDir, "worker-results", "evidence.jsonl"),
    JSON.stringify({
      id: "ev-bad-span",
      kind: "observation",
      summary: "cites lines that do not exist in the raw file",
      source_ids: ["raw:subagents/fake.md:900-950"],
      source_refs: [{ source_id: "raw:subagents/fake.md:900-950", path: "raw/subagents/fake.md", kind: "raw", hash, hash_alg: "fnv1a-64", span: "900-950", observed_at: observedAt }],
      observed_at: observedAt,
    }) + "\n",
    "utf8",
  );
  runNode(["driver.mjs", "--run-dir", runDir]);
  const gate = runNode(["scripts/strict-gate.mjs", "--run-dir", runDir], { env: { ...process.env, RECEIPTS_MIN_AGENT_COVERAGE: "1" } });
  assert.notEqual(gate.status, 0);
  assert.ok(gate.stdout.includes("outside file line range"), `bad raw span must be rejected; got ${gate.stdout}`);
});

test("lane digests are conservative and receipts next renders the brief", (t) => {
  const runDir = freshRunDir("digests-next");
  t.after(() => removeDir(runDir));

  // Passing receipt for a label, then a lane that rides the LABEL (never the
  // receipt id) - its attestation is label-backed, so the digest must floor
  // at read-unverified, never skip-verified.
  const mint = coreBin(["run", "--run-dir", runDir, "--lane", "orchestrator", "--agent-id", "prime", "--label", "test:demo", "--", "node", "-e", "process.exit(0)"]);
  assert.equal(mint.status, 0, mint.stderr);
  const laneFile = path.join(runDir, "raw", "subagents", "rider.md");
  fs.writeFileSync(
    laneFile,
    ["```receipts-evidence-jsonl", JSON.stringify({ id: "ev-label-rider", kind: "observation", summary: "the demo check passes for my change", source_ids: ["test:demo"], observed_at: now() }), "```", ""].join("\n"),
    "utf8",
  );
  assert.equal(
    runNode(["scripts/ingest-subagent.mjs", "--run-dir", runDir, "--lane", "rider", "--agent-id", "rider-agent", "--from", laneFile]).status,
    0,
  );
  assert.equal(runNode(["driver.mjs", "--run-dir", runDir]).status, 0);

  const packet = JSON.parse(fs.readFileSync(path.join(runDir, "state", "next_pass_packet.json"), "utf8"));
  const riderDigest = packet.lane_digests.find((d) => d.lane === "rider");
  assert.ok(riderDigest, `rider digest must exist; got ${JSON.stringify(packet.lane_digests)}`);
  assert.equal(riderDigest.attested, 1, "label-backed claim still counts attested");
  assert.equal(
    riderDigest.read_recommendation,
    "read-unverified",
    "label-backed attestation must floor at read-unverified (review finding 5)",
  );
  assert.ok(riderDigest.drill_down.some((h) => /^raw:subagents\/rider\.md:\d+-\d+$/.test(h)), `drill handles must be span-suffixed; got ${JSON.stringify(riderDigest.drill_down)}`);
  const orchestratorDigest = packet.lane_digests.find((d) => d.lane === "orchestrator");
  assert.equal(orchestratorDigest.read_recommendation, "skip-verified", "infra-only receipt lane is skippable");

  // The brief.
  const brief = coreBin(["next", "--run-dir", runDir]);
  assert.equal(brief.status, 0, brief.stderr);
  assert.ok(brief.stdout.includes("RECEIPTS BRIEF"), brief.stdout);
  assert.ok(brief.stdout.includes("WORKLIST"), "brief must lead with the worklist");
  assert.ok(brief.stdout.includes("rider [read-unverified]"), `brief must render digests; got ${brief.stdout}`);
  assert.ok(brief.stdout.includes("drill: raw:subagents/rider.md:"), "brief must expose drill handles");
  assert.ok(brief.stdout.includes("DRIFT: unknown - run receipts gate"), "drift placeholder without a gate report");

  const briefJson = coreBin(["next", "--run-dir", runDir, "--json"]);
  assert.equal(briefJson.status, 0, briefJson.stderr);
  const parsed = JSON.parse(briefJson.stdout);
  assert.ok(Array.isArray(parsed.lane_digests) && parsed.lane_digests.length >= 2);
  assert.equal(parsed.verdict, "gate-not-recorded");
});
