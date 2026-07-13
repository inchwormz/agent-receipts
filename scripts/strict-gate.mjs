#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function usage() {
  return [
    "Usage:",
    "  node scripts/strict-gate.mjs --run-dir <path>",
    "",
    "Fails unless a substantive Receipts run has completed the explicit-state",
    "subagent -> evidence/findings/raw -> recompile -> recorded synthesis loop.",
  ].join("\n");
}

function fail(message, code = 1) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function parseArgs(argv) {
  const runDirFlag = argv.indexOf("--run-dir");
  const runDir = runDirFlag === -1 ? null : argv[runDirFlag + 1];
  if (!runDir || runDir.startsWith("--")) fail(usage(), 2);
  return { runDir: path.resolve(runDir) };
}

function readJson(file, errors, label) {
  if (!fs.existsSync(file)) {
    errors.push(`missing ${label}: ${file}`);
    return null;
  }
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    errors.push(`invalid ${label}: ${error instanceof Error ? error.message : String(error)}`);
    return null;
  }
}

function readJsonl(file, errors, label) {
  if (!fs.existsSync(file)) {
    errors.push(`missing ${label}: ${file}`);
    return [];
  }
  const records = [];
  const lines = fs.readFileSync(file, "utf8").split(/\r?\n/);
  for (const [index, line] of lines.entries()) {
    if (!line.trim()) continue;
    try {
      records.push(JSON.parse(line));
    } catch (error) {
      errors.push(
        `invalid ${label} line ${index + 1}: ${error instanceof Error ? error.message : String(error)}`,
      );
    }
  }
  return records;
}

function walkFiles(dir) {
  if (!fs.existsSync(dir)) return [];
  const files = [];
  const stack = [dir];
  while (stack.length) {
    const current = stack.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else {
        files.push(full);
      }
    }
  }
  return files;
}

function inputFiles(runDir) {
  const inputDirs = ["raw", "worker-results", "verifier-results", "receipts", "decisions", "checks"];
  return [
    path.join(runDir, "manifest.json"),
    path.join(runDir, "task.md"),
    ...inputDirs.flatMap((dir) => walkFiles(path.join(runDir, dir))),
  ].filter((file) => fs.existsSync(file)).sort();
}

function inputFingerprint(runDir) {
  return inputFiles(runDir).map((file) => {
    const stat = fs.statSync(file);
    return {
      path: path.relative(runDir, file).replace(/\\/g, "/"),
      size: stat.size,
      hash: fnv1aHash(fs.readFileSync(file)),
    };
  });
}

// F12: the input fingerprint is mandatory. No fingerprint = fail-closed;
// the old mtime heuristic is gone (mtimes are not evidence).
function stalePacket(runDir) {
  const fingerprintPath = path.join(runDir, "state", "input_fingerprint.json");
  if (!fs.existsSync(fingerprintPath)) return "missing";
  const stored = JSON.stringify(readJson(fingerprintPath, [], "input_fingerprint"));
  return stored !== JSON.stringify(inputFingerprint(runDir)) ? "stale" : null;
}

function hasSourceIds(records) {
  return records.every((record) => Array.isArray(record.source_ids) && record.source_ids.length > 0);
}

function sourceRefs(record) {
  return Array.isArray(record.source_refs) ? record.source_refs : [];
}

function isDirectSourceId(sourceId) {
  return typeof sourceId === "string" && /^(file|command|test|log):/.test(sourceId);
}

const ALLOWED_SOURCE_KINDS = new Set([
  "file",
  "commit",
  "raw",
  "command",
  "test",
  "log",
  "packet",
  "verifier",
  "evidence",
  "objective",
  "receipt",
]);

// Schema 2 claim promotion is engine-owned and bound to an exact check
// attempt. Receipt ids and caller-selected labels are event attribution only.
let VERIFIED_CLAIMS = new Set();

const MAX_OBSERVED_AT_DRIFT_DAYS = 7;

function checkSourceKind(source, errors, prefix) {
  if (!source.kind) return;
  if (!ALLOWED_SOURCE_KINDS.has(source.kind)) {
    errors.push(
      `${prefix} source_ref ${source.source_id ?? "<unknown>"} kind "${source.kind}" is not in the allowed set (${[...ALLOWED_SOURCE_KINDS].join("|")})`,
    );
  }
}

function checkObservedAtWindow(observedAt, anchorMs, errors, prefix, label) {
  if (!observedAt || anchorMs === null) return;
  const ts = Date.parse(observedAt);
  if (Number.isNaN(ts)) {
    errors.push(`${prefix} ${label} "${observedAt}" is not a valid ISO8601 timestamp`);
    return;
  }
  const driftDays = Math.abs(ts - anchorMs) / 86_400_000;
  if (driftDays > MAX_OBSERVED_AT_DRIFT_DAYS) {
    errors.push(
      `${prefix} ${label} "${observedAt}" is outside the ${MAX_OBSERVED_AT_DRIFT_DAYS}-day window around run created_at`,
    );
  }
}

function fnv1aHash(buffer) {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const byte of buffer) {
    hash ^= BigInt(byte);
    hash = (hash * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

// F3: file citations resolve against run_dir first, then the manifest's
// recorded repo_root. There is deliberately no fallback to this package's own
// directory — that fallback made citations verify against the wrong repo.
let REPO_ROOT = null;

function resolveSourcePath(runDir, sourcePath) {
  if (!sourcePath) return null;
  if (path.isAbsolute(sourcePath)) return sourcePath;

  const insideRun = path.resolve(runDir, sourcePath);
  if (fs.existsSync(insideRun)) return insideRun;

  return REPO_ROOT ? path.resolve(REPO_ROOT, sourcePath) : null;
}

function checkHashAlg(source, errors, prefix) {
  if (source.hash_alg && source.hash_alg !== "fnv1a-64") {
    errors.push(
      `${prefix} source_ref ${source.source_id} uses unsupported hash_alg "${source.hash_alg}" (expected "fnv1a-64")`,
    );
  }
  // Require a hash for every source_ref regardless of kind. Previously the hash
  // format check only fired when hash was truthy, which let command/test/log
  // kind refs omit the hash entirely and bypass integrity verification.
  if (source.hash === undefined || source.hash === null || source.hash === "") {
    errors.push(
      `${prefix} source_ref ${source.source_id ?? "<unknown>"} is missing hash (required for every source_ref)`,
    );
  } else if (!/^[0-9a-f]{16}$/.test(String(source.hash))) {
    errors.push(
      `${prefix} source_ref ${source.source_id} hash "${source.hash}" is not a valid fnv1a-64 digest`,
    );
  }
}

// G1/G6/G10: defense-in-depth against absolute or machine-specific paths
// leaking into a packet. Ingest normalizes abs-inside-repo paths to repo-
// relative, but a hand-crafted packet or an agent that bypassed ingest could
// still carry them. Reject any `file:` source_id with a drive letter or
// POSIX-absolute path, and reject backslashes — both break determinism
// across machines.
function checkMachineSpecificPath(source, errors, prefix) {
  const sourceId = typeof source.source_id === "string" ? source.source_id : "";
  const sourcePath = typeof source.path === "string" ? source.path : "";
  const fileIdMatch = /^file:(.+?):(?:\d+(?:-\d+)?)$/.exec(sourceId);
  const idPath = fileIdMatch ? fileIdMatch[1] : "";
  const looksAbsolute = (candidate) =>
    /^[A-Za-z]:[\\/]/.test(candidate) || candidate.startsWith("/");
  const hasBackslash = (candidate) => candidate.includes("\\");
  if (looksAbsolute(idPath)) {
    errors.push(
      `${prefix} source_id ${sourceId} embeds a machine-specific absolute path; must be repo-relative`,
    );
  }
  if (source.kind === "file" && looksAbsolute(sourcePath)) {
    errors.push(
      `${prefix} file source_ref ${source.source_id ?? "<unknown>"} path must be repo-relative; got machine-specific "${sourcePath}"`,
    );
  }
  if (source.kind === "file" && hasBackslash(sourcePath)) {
    errors.push(
      `${prefix} file source_ref ${source.source_id ?? "<unknown>"} path must use forward slashes; got "${sourcePath}"`,
    );
  }
}

function checkFileSourceRef(source, runDir, errors, prefix, warnings = null) {
  if (source.kind !== "file") return;

  const resolved = resolveSourcePath(runDir, source.path);
  if (!resolved || !fs.existsSync(resolved)) {
    errors.push(`${prefix} file source_ref ${source.source_id} path does not exist: ${source.path}`);
    return;
  }

  const bytes = fs.readFileSync(resolved);
  const actualHash = fnv1aHash(bytes);
  if (source.hash !== actualHash) {
    // CUSTODY vs DRIFT: mismatches inside the run dir are tampering (error);
    // project-tree citations legitimately drift when fixes land after review
    // (warning - the ingest-time hash pins what the agent actually saw).
    const insideRun = (path.resolve(resolved) + path.sep).startsWith(path.resolve(runDir) + path.sep);
    if (insideRun) {
      errors.push(
        `${prefix} file source_ref ${source.source_id} hash mismatch inside the run dir: expected ${actualHash}, got ${source.hash} - quarantined artifacts are immutable`,
      );
    } else if (Array.isArray(warnings)) {
      warnings.push(
        `${prefix} citation drifted: ${source.source_id} was verified at ingest but the file has since changed (post-review fixes are the usual cause)`,
      );
    }
    return; // skip span validation against a moved file either way
  }

  if (source.span) {
    const lineCount = bytes.toString("utf8").split(/\r?\n/).length;
    const match = /^(\d+)(?:-(\d+))?$/.exec(String(source.span));
    if (!match) {
      errors.push(`${prefix} file source_ref ${source.source_id} span must be a line or line range`);
      return;
    }
    const start = Number(match[1]);
    const end = Number(match[2] ?? match[1]);
    if (start < 1 || end < start || end > lineCount) {
      errors.push(
        `${prefix} file source_ref ${source.source_id} span ${source.span} is outside file line range 1-${lineCount}`,
      );
    }
  }
}

function checkRawSourceRef(source, runDir, errors, prefix) {
  if (source.kind !== "raw") return;
  // Raw artifacts are always resolved relative to run_dir. Absolute legacy
  // paths are tolerated for backwards compatibility, but we re-verify the
  // hash so quarantined subagent output cannot be mutated after ingest.
  const candidate = path.isAbsolute(source.path) ? source.path : path.resolve(runDir, source.path);
  if (!fs.existsSync(candidate)) {
    errors.push(
      `${prefix} raw source_ref ${source.source_id} path does not exist under run_dir: ${source.path}`,
    );
    return;
  }
  const bytes = fs.readFileSync(candidate);
  const actualHash = fnv1aHash(bytes);
  if (source.hash !== actualHash) {
    errors.push(
      `${prefix} raw source_ref ${source.source_id} hash mismatch: expected ${actualHash}, got ${source.hash}`,
    );
    return;
  }
  // Phase 3: drill-down spans on raw refs must point at lines that exist -
  // a wrong pointer is worse than none (review finding 9).
  if (source.span) {
    const match = /^(\d+)(?:-(\d+))?$/.exec(String(source.span));
    if (!match) {
      errors.push(`${prefix} raw source_ref ${source.source_id} span must be a line or line range`);
      return;
    }
    const lineCount = bytes.toString("utf8").split(/\r?\n/).length;
    const start = Number(match[1]);
    const end = Number(match[2] ?? match[1]);
    if (start < 1 || end < start || end > lineCount) {
      errors.push(
        `${prefix} raw source_ref ${source.source_id} span ${source.span} is outside file line range 1-${lineCount}`,
      );
    }
  }
}

// F2 + schema 2 truth-in-labeling: a direct source is content-backed file/git
// evidence or a compiler-verified, current claim binding. Runtime receipt ids
// and labels prove that a command ran, not that a semantic claim is true.
function hasDirectSource(record) {
  if (VERIFIED_CLAIMS.has(record.id)) return true;
  if (
    sourceRefs(record).some(
      (source) => source && source.kind === "file" && source.hash_basis !== "label",
    )
  ) {
    return true;
  }
  return (record.source_ids ?? []).some((id) => {
    if (typeof id !== "string") return false;
    // A commit citation is content-addressed by git itself; the gate
    // re-verifies existence rather than trusting ingest's stamp.
    if (id.startsWith("commit:")) return commitExistsInRepo(id.slice("commit:".length));
    return false;
  });
}

const COMMIT_CACHE = new Map();
function commitExistsInRepo(sha) {
  if (!REPO_ROOT || !/^[0-9a-f]{7,40}$/.test(sha)) return false;
  if (COMMIT_CACHE.has(sha)) return COMMIT_CACHE.get(sha);
  let ok = false;
  try {
    execFileSync("git", ["-C", REPO_ROOT, "cat-file", "-e", `${sha}^{commit}`], { stdio: "ignore" });
    ok = true;
  } catch {
    ok = false;
  }
  COMMIT_CACHE.set(sha, ok);
  return ok;
}

function packetSourceIds(packet) {
  return new Set((packet?.sources ?? []).map((source) => source.source_id).filter(Boolean));
}

function checkDeclaredSourceRefs(records, runDir, errors, label, anchorMs, warnings) {
  for (const record of records) {
    const declared = sourceRefs(record);
    const declaredIds = new Set(declared.map((source) => source.source_id).filter(Boolean));
    for (const sourceId of record.source_ids ?? []) {
      if (isDirectSourceId(sourceId) && !declaredIds.has(sourceId)) {
        errors.push(
          `${label} ${record.id ?? "<unknown>"} lists direct source_id ${sourceId} without matching source_refs entry`,
        );
      }
    }
    if (declared.length === 0) continue;

    const sourceIds = new Set(record.source_ids ?? []);
    for (const source of declared) {
      const prefix = `${label} ${record.id ?? "<unknown>"}`;
      if (!source.source_id) errors.push(`${prefix} has source_ref without source_id`);
      if (!source.path) errors.push(`${prefix} has source_ref ${source.source_id ?? "<unknown>"} without path`);
      if (!source.kind) errors.push(`${prefix} has source_ref ${source.source_id ?? "<unknown>"} without kind`);
      if (!source.hash) errors.push(`${prefix} has source_ref ${source.source_id ?? "<unknown>"} without hash`);
      if (!source.observed_at) {
        errors.push(`${prefix} has source_ref ${source.source_id ?? "<unknown>"} without observed_at`);
      }
      if (source.source_id && !sourceIds.has(source.source_id)) {
        errors.push(`${prefix} declares source_ref ${source.source_id} but does not list it in source_ids`);
      }
      checkSourceKind(source, errors, prefix);
      checkHashAlg(source, errors, prefix);
      checkMachineSpecificPath(source, errors, prefix);
      checkFileSourceRef(source, runDir, errors, prefix, warnings);
      checkRawSourceRef(source, runDir, errors, prefix);
      checkObservedAtWindow(source.observed_at, anchorMs, errors, prefix, "source_ref observed_at");
    }
    checkObservedAtWindow(
      record.observed_at,
      anchorMs,
      errors,
      `${label} ${record.id ?? "<unknown>"}`,
      "record observed_at",
    );
  }
}

function checkPacketSourceIntegrity(packet, runDir, errors, anchorMs, warnings) {
  // Packet.sources is synthesized by the compiler. Its `kind` field is the
  // record's category (e.g. "evidence", "verifier") OR — for legacy synthesis
  // paths — the evidence record's own semantic kind (e.g. "root-cause",
  // "subagent-session"). Do NOT enforce the source_ref kind allowlist on
  // packet.sources; that check is reserved for user-authored source_refs on
  // evidence/verifier records.
  for (const source of packet?.sources ?? []) {
    const prefix = `packet sources ${source.source_id ?? "<unknown>"}`;
    checkHashAlg(source, errors, prefix);
    checkMachineSpecificPath(source, errors, prefix);
    checkFileSourceRef(source, runDir, errors, prefix, warnings);
    checkRawSourceRef(source, runDir, errors, prefix);
    checkObservedAtWindow(source.observed_at, anchorMs, errors, prefix, "observed_at");
  }
}

// G5: contradictions may carry optional source_refs populated from the
// union of the conflicting evidence records' refs restricted to shared direct
// spans. When present, verify each ref like any other source_ref — that way
// tampered files cited by a contradiction are detectable.
function checkContradictionSourceRefs(packet, runDir, errors, anchorMs, warnings) {
  for (const contradiction of packet?.contradictions ?? []) {
    const refs = Array.isArray(contradiction.source_refs) ? contradiction.source_refs : [];
    for (const source of refs) {
      const prefix = `contradiction ${contradiction.id ?? "<unknown>"}`;
      checkHashAlg(source, errors, prefix);
      checkMachineSpecificPath(source, errors, prefix);
      checkFileSourceRef(source, runDir, errors, prefix, warnings);
      checkRawSourceRef(source, runDir, errors, prefix);
      checkObservedAtWindow(source.observed_at, anchorMs, errors, prefix, "source_ref observed_at");
    }
  }
}

const KNOWN_SCHEMA_VERSIONS = new Set(["1.1.0", "1.2.0", "2.0.0"]);

function checkPacketSchemaVersion(packet, errors) {
  if (!packet) return;
  if (!KNOWN_SCHEMA_VERSIONS.has(packet.schema_version)) {
    errors.push(
      `packet schema_version "${packet.schema_version ?? "<missing>"}" is not in the known set (${[...KNOWN_SCHEMA_VERSIONS].join("|")})`,
    );
  }
}

function checkCompiledSourceRefs(records, packet, errors, label) {
  const compiled = packetSourceIds(packet);
  for (const record of records) {
    for (const source of sourceRefs(record)) {
      if (source.source_id && !compiled.has(source.source_id)) {
        errors.push(
          `${label} ${record.id ?? "<unknown>"} source_ref ${source.source_id} was not compiled into next_pass_packet.sources`,
        );
      }
    }
    // Catch dangling direct source_ids: ids that are declared on the record but
    // never promoted into packet.sources (e.g. because the matching source_ref
    // was silently dropped). A direct source_id with no compiled counterpart is
    // laundering — fail the gate.
    for (const sourceId of record.source_ids ?? []) {
      if (isDirectSourceId(sourceId) && !compiled.has(sourceId)) {
        errors.push(
          `${label} ${record.id ?? "<unknown>"} direct source_id ${sourceId} was not compiled into next_pass_packet.sources`,
        );
      }
    }
  }
}

function requiresDirectEvidence(record) {
  const kind = String(record.kind ?? "").toLowerCase();
  // `blocker` records by design have no direct file/command provenance —
  // the block IS the evidence; the raw/subagents/* cite is the artifact.
  // `unstructured` records are quarantined prose fallbacks: already demoted,
  // already labeled unverified — requiring provenance would just re-crash
  // lanes the forgiving parser deliberately kept alive.
  // `receipt` records are runtime-authored from the verified journal - they
  // ARE the provenance.
  if (["objective", "process", "subagent-session", "codex-synthesis", "blocker", "unstructured", "receipt", "work"].includes(kind)) return false;
  return true;
}

// F6: exemptions are TYPED, never keyed on free text — an agent naming its
// finding "vf-subagent-anything" must not relax requirements. Infrastructure
// findings carry a finding_kind stamped by the driver/fixtures; a
// closure_reason waives the direct-ref requirement only when the finding
// cites the raw:* artifact that records the bounding decision.
const EXEMPT_FINDING_KINDS = new Set(["synthesis", "subagent-session", "bootstrap"]);

function requiresDirectVerifier(record) {
  const findingKind = String(record.finding_kind ?? "").toLowerCase();
  if (EXEMPT_FINDING_KINDS.has(findingKind)) return false;
  if (typeof record.closure_reason === "string" && record.closure_reason.trim().length > 0) {
    const citesRaw = (record.source_ids ?? []).some(
      (id) => typeof id === "string" && id.startsWith("raw:"),
    );
    return !citesRaw;
  }
  // verifier_score is caller-authored diagnostic metadata. It never changes
  // whether a semantic pass needs direct provenance.
  return String(record.status ?? "").toLowerCase() === "passed";
}

function statusSummary(records) {
  return records.map((record) => ({
    id: record.id ?? "<unknown>",
    status: record.status ?? "<missing>",
    verifier_score: record.verifier_score ?? null,
  }));
}

// G8: Known evidence kinds pulled from prior Receipts runs plus the obvious
// standard kinds. Records whose `kind` is outside this set get a soft warning
// so typos or one-off improvisations (e.g. "fictional-kind") are surfaced
// without failing the gate. Adding a new kind is as simple as appending to
// this set.
const KNOWN_EVIDENCE_KINDS = new Set([
  "objective",
  "subagent-session",
  "codex-synthesis",
  "observation",
  "code-change",
  "test-change",
  "root-cause",
  "blocker",
  "symptom",
  "measurement",
  "failure-mode",
  "gap",
  "missing-check",
  "proposal",
  "risk",
  "architecture",
  "data-loss",
  "code-defect",
  "schema-gap",
  "contract-gap",
  "gate-gap",
  "process-gap",
  "plan",
  "unstructured",
  "receipt",
  "work",
]);

function checkUnknownEvidenceKinds(evidence, warnings) {
  for (const record of evidence) {
    const kind = String(record.kind ?? "").trim();
    if (!kind) continue;
    if (!KNOWN_EVIDENCE_KINDS.has(kind)) {
      warnings.push(
        `evidence record ${record.id ?? "<unknown>"} uses unknown kind '${kind}' — not rejected, but consider adding to the known-kinds set`,
      );
    }
  }
}

// R5: Every evidence record whose kind is NOT in the infrastructure set
// (objective/subagent-session/codex-synthesis) must have at least one
// `source_ids` entry that references raw/subagents/* directly OR matches the
// `source_ids` of a `subagent-session` record. This closes the "Prime invented
// evidence outside any lane" laundering path.
function checkSubagentTraceability(evidence, errors) {
  const infrastructureKinds = new Set(["objective", "subagent-session", "codex-synthesis", "receipt", "work"]);
  const sessionSourceIds = new Set();
  for (const record of evidence) {
    if (record.kind === "subagent-session") {
      for (const sourceId of record.source_ids ?? []) sessionSourceIds.add(sourceId);
    }
  }
  const offenders = [];
  for (const record of evidence) {
    const kind = String(record.kind ?? "").toLowerCase();
    if (infrastructureKinds.has(kind)) continue;
    const sourceIds = Array.isArray(record.source_ids) ? record.source_ids : [];
    const hasRawSubagent = sourceIds.some((id) => typeof id === "string" && id.startsWith("raw:subagents/"));
    const hasSessionOverlap = sourceIds.some((id) => sessionSourceIds.has(id));
    if (!hasRawSubagent && !hasSessionOverlap) {
      offenders.push(record.id ?? "<unknown>");
    }
  }
  if (offenders.length > 0) {
    errors.push(
      `evidence records lack subagent-session traceability (expected raw:subagents/* or overlap with a subagent-session record): ${offenders.join(", ")}`,
    );
  }
}

// R6 + F2: For evidence kinds that assert concrete change/causation/tests
// (code-change, root-cause, test-change), require at least one CONTENT-hashed
// anchor. Until receipts land (M2), that means a file ref hashed from disk;
// command/test refs carry label-derived hashes and do not count — a claim
// that "the tests ran" is unattested until a receipt exists.
function checkDirectSourceRefRatio(evidence, errors) {
  const requiredKinds = new Set(["code-change", "root-cause", "test-change"]);
  const offenders = [];
  for (const record of evidence) {
    const kind = String(record.kind ?? "").toLowerCase();
    if (!requiredKinds.has(kind)) continue;
    const refs = Array.isArray(record.source_refs) ? record.source_refs : [];
    const hasDirect = refs.some(
      (ref) => ref && ref.kind === "file" && ref.hash_basis !== "label",
    );
    if (!hasDirect) offenders.push(record.id ?? "<unknown>");
  }
  if (offenders.length > 0) {
    errors.push(
      `evidence records of kind code-change/root-cause/test-change lack a content-hashed file source_ref (label-hashed command/test refs are not provenance): ${offenders.join(", ")}`,
    );
  }
}

// R7: Count distinct agent_id values across evidence (excluding the seed
// objective record and auto ev-subagent-session-* records). Require at least
// RECEIPTS_MIN_AGENT_COVERAGE distinct non-empty agent_ids. Skip for the
// readiness-fixture packet (pass-0001 with only the pending synthesis
// finding) so bootstrap scenarios still pass.
function checkAgentCoverage(evidence, findings, manifest, packet, errors) {
  const env = process.env.RECEIPTS_MIN_AGENT_COVERAGE;
  const parsed = env !== undefined && env !== "" ? Number(env) : NaN;
  const minCoverage = Number.isFinite(parsed) && parsed > 0 ? parsed : 3;

  // Readiness-fixture escape hatch: pass-0001 with only vf-codex-synthesis-pending.
  const passId = packet?.pass_id ?? manifest?.pass_id;
  const onlyPendingFinding =
    Array.isArray(findings) &&
    findings.length === 1 &&
    findings[0]?.id === "vf-codex-synthesis-pending";
  if (passId === "pass-0001" && onlyPendingFinding) return;

  const distinct = new Set();
  for (const record of evidence) {
    if (record.id === "ev-objective") continue;
    if (typeof record.id === "string" && record.id.startsWith("ev-subagent-session-")) continue;
    // Unstructured prose fallbacks are not substantive contributions — three
    // prose-only lanes must not satisfy the diversity floor. Runtime receipt
    // records are the orchestrator's, not a lane's.
    if (["unstructured", "receipt", "work"].includes(String(record.kind ?? ""))) continue;
    const agentId = record.agent_id;
    if (typeof agentId === "string" && agentId.trim().length > 0) {
      distinct.add(agentId.trim());
    }
  }
  if (distinct.size < minCoverage) {
    errors.push(
      `agent-id coverage floor not met: saw ${distinct.size} distinct agent_id(s) across evidence, need at least ${minCoverage} (set RECEIPTS_MIN_AGENT_COVERAGE to override)`,
    );
  }
}

function main() {
  const { runDir } = parseArgs(process.argv.slice(2));
  const errors = [];
  const warnings = [];

  if (!fs.existsSync(runDir)) fail(`run dir does not exist: ${runDir}`);

  const manifestPath = path.join(runDir, "manifest.json");
  const packetPath = path.join(runDir, "state", "next_pass_packet.json");
  const evidencePath = path.join(runDir, "worker-results", "evidence.jsonl");
  const findingsPath = path.join(runDir, "verifier-results", "findings.jsonl");
  const rawDir = path.join(runDir, "raw");
  const rawSubagentDir = path.join(rawDir, "subagents");

  const manifest = readJson(manifestPath, errors, "manifest");
  const packet = readJson(packetPath, errors, "next_pass_packet");
  const evidence = readJsonl(evidencePath, errors, "evidence");
  const findings = readJsonl(findingsPath, errors, "verifier findings");

  if (manifest && packet && manifest.pass_id !== packet.pass_id) {
    errors.push(`manifest pass_id ${manifest.pass_id} does not match packet pass_id ${packet.pass_id}`);
  }

  if (manifest?.pass_id === "pass-0001") {
    errors.push("run is still pass-0001; strict gate requires at least one promoted recurrence pass");
  }

  REPO_ROOT =
    typeof manifest?.repo_root === "string" && manifest.repo_root.length > 0
      ? manifest.repo_root
      : null;
  VERIFIED_CLAIMS = new Set(
    (packet?.trust_assessments ?? [])
      .filter(
        (assessment) =>
          assessment.applicability === "current" &&
          ["verified", "verifier_backed"].includes(assessment.claim_status),
      )
      .map((assessment) => assessment.subject_id),
  );

  const staleness = stalePacket(runDir);
  if (staleness === "missing") {
    errors.push(
      "state/input_fingerprint.json is missing; recompile with the receipts-core compiler (fail-closed: no fingerprint, no gate)",
    );
  } else if (staleness === "stale") {
    errors.push("next_pass_packet.json is stale; re-run driver.mjs --run-dir before proceeding");
  }

  if (evidence.length <= 1) {
    errors.push("run has only objective evidence; subagent/compiler promotion has not happened");
  }

  checkPacketSchemaVersion(packet, errors);
  const anchorMs = manifest?.created_at ? Date.parse(manifest.created_at) : null;
  const anchor = Number.isFinite(anchorMs) ? anchorMs : null;
  if (!hasSourceIds(evidence)) errors.push("one or more evidence records lack source_ids");
  if (!hasSourceIds(findings)) errors.push("one or more verifier findings lack source_ids");
  checkDeclaredSourceRefs(evidence, runDir, errors, "evidence", anchor, warnings);
  checkDeclaredSourceRefs(findings, runDir, errors, "verifier finding", anchor, warnings);
  checkCompiledSourceRefs(evidence, packet, errors, "evidence");
  checkCompiledSourceRefs(findings, packet, errors, "verifier finding");
  checkPacketSourceIntegrity(packet, runDir, errors, anchor, warnings);
  checkContradictionSourceRefs(packet, runDir, errors, anchor, warnings);
  checkSubagentTraceability(evidence, errors);
  checkDirectSourceRefRatio(evidence, errors);
  checkAgentCoverage(evidence, findings, manifest, packet, errors);
  // G8: soft-warn on evidence records with kinds outside the known set.
  checkUnknownEvidenceKinds(evidence, warnings);

  // Records already carrying provenance warnings are DEMOTED (never facts,
  // digest-flagged, advisory-worklisted) - that is their penalty. Red-carding
  // them again here is double jeopardy with no resolution channel (field
  // find, 2026-07-13: a harvested prose line citing a not-yet-existing file
  // was demoted correctly, then red-carded the whole run). The summary-only
  // red is reserved for records CLAIMING substance with clean provenance.
  const summaryOnlyEvidence = evidence.filter(
    (record) =>
      requiresDirectEvidence(record) &&
      !hasDirectSource(record) &&
      !(record.provenance_warnings ?? []).length,
  );
  if (summaryOnlyEvidence.length > 0) {
    errors.push(
      `summary-only evidence lacks direct file/command provenance: ${summaryOnlyEvidence
        .map((record) => record.id)
        .join(", ")}`,
    );
  }

  const summaryOnlyFindings = findings.filter(
    (record) => requiresDirectVerifier(record) && !hasDirectSource(record),
  );
  if (summaryOnlyFindings.length > 0) {
    errors.push(
      `summary-only verifier findings lack direct file/command provenance: ${summaryOnlyFindings
        .map((record) => record.id)
        .join(", ")}`,
    );
  }

  // F6: the "did subagents actually run" check is typed — it counts
  // kind:"subagent-session" records (which only mechanical ingest emits),
  // never text that merely mentions subagents.
  const subagentSessionEvidence = evidence.filter((record) => record.kind === "subagent-session");
  const rawSubagentFiles = walkFiles(rawSubagentDir);
  const compiledSources = packetSourceIds(packet);
  if (rawSubagentFiles.length === 0) {
    errors.push("no raw/subagents session artifacts found; subagent outputs were not quarantined");
  }
  if (rawSubagentFiles.length > 0) {
    const hasCompiledRawSubagent = [...compiledSources].some((sourceId) => sourceId.startsWith("raw:subagents/"));
    if (!hasCompiledRawSubagent) {
      errors.push("raw/subagents artifacts were not compiled into next_pass_packet.sources");
    }
  }
  if (subagentSessionEvidence.length === 0) {
    errors.push("no subagent-session evidence records found; raw subagent output was not ingested mechanically");
  }
  // G2: require every subagent-session record to carry non-empty agent_id AND
  // lane so consumers can attribute the session to a specific lane. Historic
  // packets (schema_version < 1.1.0) predate this stamping, so skip the check
  // for those runs; all current runs use 1.1.0 and hit the enforcement path.
  // Set-membership, never lexicographic string comparison (breaks at 1.10.0).
  const schemaVersion = packet?.schema_version ?? null;
  const enforceG2 = schemaVersion === null || KNOWN_SCHEMA_VERSIONS.has(schemaVersion);
  if (enforceG2) {
    const missingAttribution = subagentSessionEvidence
      .filter((record) => {
        const agentOk = typeof record.agent_id === "string" && record.agent_id.trim().length > 0;
        const laneOk = typeof record.lane === "string" && record.lane.trim().length > 0;
        return !agentOk || !laneOk;
      })
      .map((record) => record.id ?? "<unknown>");
    if (missingAttribution.length > 0) {
      errors.push(
        `G2: subagent-session evidence records must carry non-empty agent_id AND lane (offenders: ${missingAttribution.join(", ")})`,
      );
    }
  }
  for (const record of subagentSessionEvidence) {
    const refsRawSubagent = (record.source_ids ?? []).some((sourceId) => sourceId.startsWith("raw:subagents/"));
    if (!refsRawSubagent) {
      errors.push(`subagent-session evidence ${record.id ?? "<unknown>"} does not reference raw:subagents/*`);
    }
    const rawSourceRefs = sourceRefs(record).filter((source) => source.source_id?.startsWith("raw:subagents/"));
    if (rawSourceRefs.length === 0) {
      errors.push(`subagent-session evidence ${record.id ?? "<unknown>"} does not declare raw:subagents/* source_refs`);
    }
    for (const source of rawSourceRefs) {
      if (!compiledSources.has(source.source_id)) {
        errors.push(
          `subagent-session evidence ${record.id ?? "<unknown>"} raw source_ref ${source.source_id} was not compiled into next_pass_packet.sources`,
        );
      }
    }
  }

  const hasSynthesisEvidence = evidence.some((record) => record.kind === "codex-synthesis");
  const hasSynthesisRaw = walkFiles(rawDir).some((file) => path.basename(file).startsWith("codex-synthesis-"));
  if (!hasSynthesisEvidence || !hasSynthesisRaw) {
    errors.push("Codex synthesis was not recorded through driver.mjs --record-synthesis");
  }

  const nonPassingFindings = findings.filter((record) => record.status !== "passed");
  if (nonPassingFindings.length > 0) {
    errors.push(`non-passing verifier findings remain: ${nonPassingFindings.map((record) => record.id).join(", ")}`);
  }

  // M2: receipt refutations are non-negotiable reds. The compiler detected a
  // passed claim whose cited check actually FAILED when executed.
  for (const contradiction of packet?.contradictions ?? []) {
    if (String(contradiction.id ?? "").startsWith("con:receipt:")) {
      errors.push(`refuted by execution receipt: ${contradiction.summary}`);
    }
  }

  const haltKinds = (packet?.halt_signals ?? []).map((signal) => signal.kind);
  if (!haltKinds.includes("ready-to-halt")) {
    errors.push("packet is not ready-to-halt");
  }
  // Phase 2: the compiler is the single author of blocking classification;
  // the gate consumes the flags. Blocking && !resolved = red. Advisory items
  // (verify-claim, re-task-or-accept, low/medium adjudications) never red.
  // Legacy 1.1.0 packets carry no flags - preserve their old strictness.
  const actions = packet?.candidate_actions ?? [];
  if (packet?.schema_version === "1.1.0") {
    if (actions.length > 0) {
      errors.push("packet still has candidate_actions; resolve or record them before finalizing");
    }
  } else {
    for (const item of actions) {
      if (item.blocking === true && item.resolved !== true) {
        errors.push(
          `unresolved blocking worklist item [${item.category ?? "?"}] ${item.id}: ${String(item.title ?? "").slice(0, 120)} (clear with: receipts resolve --run-dir <run-dir> --target ${item.decision_dependency_ids?.[0] ?? "<target>"} --reason "...")`,
        );
      }
    }
  }

  if ((packet?.sources ?? []).length < evidence.length + findings.length) {
    warnings.push("packet source count is lower than evidence+finding count; inspect source promotion");
  }

  const report = {
    ok: errors.length === 0,
    run_dir: runDir,
    pass_id: manifest?.pass_id ?? null,
    packet_pass_id: packet?.pass_id ?? null,
    evidence_count: evidence.length,
    verifier_findings: statusSummary(findings),
    halt_kinds: haltKinds,
    candidate_actions: (packet?.candidate_actions ?? []).length,
    errors,
    warnings,
  };

  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);

  // The verdict is not durable if it only lives on a terminal. Field
  // finding (2026-07-13 NTM runs): orchestrators run the gate bare, read
  // "ok": true, and state/gate-report.json never exists - so `next` and
  // `report` can't show the verdict and the claim itself is unverifiable
  // later. The gate now persists its own report, always.
  try {
    const stateDir = path.join(runDir, "state");
    fs.mkdirSync(stateDir, { recursive: true });
    fs.writeFileSync(path.join(stateDir, "gate-report.json"), `${JSON.stringify(report, null, 2)}\n`);
  } catch (error) {
    process.stderr.write(`strict-gate: warning: could not persist state/gate-report.json: ${error.message}\n`);
  }

  if (errors.length > 0) process.exit(1);
}

main();
