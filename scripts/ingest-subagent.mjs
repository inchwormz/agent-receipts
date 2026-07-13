#!/usr/bin/env node
// Receipts subagent ingest.
//
// Input contract (two shapes):
// 1. Fenced records. The subagent markdown contains one or both of:
//      ```receipts-evidence-jsonl  ... ```
//      ```receipts-verifier-jsonl  ... ```
//    Each fenced block is parsed one JSON record per line and appended to the
//    run-dir's worker-results/verifier-results files.
//
// 2. BLOCKED sentinel. A bare-prose subagent response that ends (or contains)
//    a line shaped `BLOCKED <reason>` is accepted even when NO fenced block is
//    present. Ingest synthesizes a `kind:"blocker"` evidence record whose
//    summary is `BLOCKED: <reason>`, whose only source_id is the quarantined
//    raw file, and exits 0. This is how subagents report unrecoverable halts
//    without forging evidence.
//
// Empty / prose-only input without a BLOCKED sentinel remains an error — the
// compiler must not absorb unprovenanced prose as evidence.
//
// Every extracted record is stamped with the caller's `--agent-id` and `--lane`
// unless the record already declares its own (subagents can override).
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

function usage() {
  return [
    "Usage:",
    "  node scripts/ingest-subagent.mjs --run-dir <path> --lane <name> --agent-id <id> --from <file>",
    "",
    "Quarantines exact subagent output as raw state, then extracts only fenced",
    "receipts-evidence-jsonl and receipts-verifier-jsonl (legacy mythos-* labels accepted) records into run files.",
    "If the input contains a `BLOCKED <reason>` sentinel and no fenced records,",
    "a synthetic blocker evidence record is emitted instead.",
  ].join("\n");
}

function fail(message, code = 1) {
  process.stderr.write(`${message}\n`);
  process.exit(code);
}

function parseArgs(argv) {
  const value = (flag) => {
    const index = argv.indexOf(flag);
    if (index === -1) return null;
    const next = argv[index + 1];
    return next && !next.startsWith("--") ? next : null;
  };
  const runDir = value("--run-dir");
  const lane = value("--lane");
  const agentId = value("--agent-id");
  const from = value("--from");
  const stdin = argv.includes("--stdin");
  if (stdin) fail("stdin subagent ingest is disabled; write output under raw/subagents and pass --from", 2);
  if (!runDir || !lane || !agentId || !from) fail(usage(), 2);
  return { runDir: path.resolve(runDir), lane, agentId, from: path.resolve(from) };
}

function slugify(input) {
  return (
    String(input)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64) || "subagent"
  );
}

function utcStamp() {
  return new Date().toISOString().replace(/[-:.]/g, "").replace(/\d{3}Z$/, "Z");
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

function readInput(args) {
  return fs.readFileSync(args.from, "utf8");
}

function isInsideDir(file, dir) {
  const relative = path.relative(dir, file);
  return relative && !relative.startsWith("..") && !path.isAbsolute(relative);
}

function normalizeRawName(rawDir, rawPath) {
  return path.relative(rawDir, rawPath).replace(/\\/g, "/");
}

// G3: advisory lock using an O_EXCL sidecar. Two or more parallel ingest
// processes writing to the same evidence.jsonl / findings.jsonl can interleave
// partial lines if the underlying write is split by the OS. Serialize the
// append through a lock file so each process sees the append as atomic from
// the file-reader's perspective. Pure stdlib: `wx` gives O_EXCL|O_CREAT, which
// fails with EEXIST when a competing process already holds the lock.
function sleepMsSync(ms) {
  const buffer = new Int32Array(new SharedArrayBuffer(4));
  Atomics.wait(buffer, 0, 0, ms);
}

function withFileLock(file, fn, { retries = 200, waitMs = 25, staleMs = 30_000 } = {}) {
  const lockFile = `${file}.lock`;
  for (let attempt = 0; attempt <= retries; attempt++) {
    try {
      const fd = fs.openSync(lockFile, "wx");
      try {
        fs.writeSync(fd, `${process.pid}:${Date.now()}`);
      } finally {
        fs.closeSync(fd);
      }
      try {
        return fn();
      } finally {
        try {
          fs.unlinkSync(lockFile);
        } catch {
          // best-effort — another process may have already reaped a stale lock
        }
      }
    } catch (err) {
      if (err && err.code !== "EEXIST") throw err;
      // If the lock looks stale (process died mid-append), reap it and retry.
      try {
        const stat = fs.statSync(lockFile);
        if (Date.now() - stat.mtimeMs > staleMs) {
          fs.unlinkSync(lockFile);
          continue;
        }
      } catch {
        // lock file vanished between EEXIST and statSync — loop and retry
      }
      sleepMsSync(waitMs);
    }
  }
  throw new Error(`timed out waiting for ${lockFile} after ${retries * waitMs}ms`);
}

function appendJsonl(file, records) {
  if (records.length === 0) return;
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const payload = `${records.map((record) => JSON.stringify(record)).join("\n")}\n`;
  withFileLock(file, () => {
    fs.appendFileSync(file, payload, "utf8");
  });
}

// ---------------------------------------------------------------------------
// FORGIVING PARSER (post-mortem 2026-07-12, run mythos-zero-visual-44 (historical)).
//
// Live NTM agents do real work and then transcribe it imperfectly: bare ```
// fences, pretty-printed JSON, arrays, single quotes, trailing commas, field
// synonyms, bare path:line citations, whole reports in prose. The first field
// test died because every one of those was a hard rejection, and each
// rejection cost a full agent round-trip until the orchestrator gave up.
//
// Principle: format is NOT the trust anchor - hashes, attribution stamping,
// and the strict gate are. So ingest is liberal in what it accepts, records
// every repair it performs, and stays exactly as strict about truth
// downstream. Nothing here weakens a single gate check.
// ---------------------------------------------------------------------------

// Fence scanner: ``` or ~~~, up to 3 leading spaces, any (or no) label.
// Classification is per-BLOCK by label when the label names receipts|mythos/evidence/
// verifier, otherwise by sniffing whether the content parses into records.
// Routing is then per-RECORD by shape (status/verifier_score => verifier), so
// an agent that dumps both kinds into one block still ingests correctly.
// 1-based line number of a character offset.
function lineOfOffset(text, offset) {
  let line = 1;
  for (let i = 0; i < offset && i < text.length; i++) {
    if (text.charCodeAt(i) === 10) line += 1;
  }
  return line;
}

function parseBlocks(text) {
  const blocks = [];
  const captured = [];
  const fence = /(?:^|\r?\n)[ ]{0,3}(```+|~~~+)([^\n]*)\r?\n([\s\S]*?)(?:^|\r?\n)[ ]{0,3}\1[ \t]*(?=\r?\n|$)/gm;
  let match;
  while ((match = fence.exec(text)) !== null) {
    const label = match[2].trim().toLowerCase();
    // Keep the body UNTRIMMED so internal line k maps to baseLine + k
    // (Phase 3 drill-down spans).
    const body = match[3];
    if (!body.trim()) continue;
    captured.push([match.index, match.index + match[0].length]);
    const bodyOffset = match.index + match[0].indexOf(body);
    const baseLine = lineOfOffset(text, bodyOffset);
    let type = null;
    if (label.includes("verifier")) type = "verifier";
    else if (label.includes("evidence")) type = "evidence";
    if (type) {
      blocks.push({ type, body, label, baseLine });
      continue;
    }
    // Unlabeled / generically-labeled block: accept only if the content
    // actually parses into record-shaped objects. Command output, code, and
    // tables fail this test and stay ignored.
    const sniff = parseRecordsFromBlock({ body, label, baseLine });
    const shaped = sniff.records.filter(
      (entry) =>
        entry.value &&
        typeof entry.value === "object" &&
        (entry.value.summary !== undefined ||
          entry.value.status !== undefined ||
          entry.value.source_ids !== undefined ||
          entry.value.text !== undefined ||
          entry.value.claim !== undefined),
    );
    if (shaped.length > 0 && shaped.length >= sniff.records.length / 2) {
      blocks.push({ type: "sniffed", body, label: label || "(bare)", baseLine });
    }
  }

  // Second-chance label scan (field-observed variant: agents wrap the label
  // in SINGLE backticks - `receipts-verifier-jsonl - so no real fence exists).
  // Wherever a receipts (or legacy mythos) evidence/verifier label appears OUTSIDE a captured
  // fence, collect the record-shaped lines that follow it. Delimiters are
  // ignored entirely; content decides.
  const labelScan = /`{0,2}(?:mythos|receipts)[-_ ](evidence|verifier)[-_ ]?jsonl`{0,2}/gi;
  const lines = text.split(/\r?\n/);
  // Precompute line start offsets to map matches to line numbers.
  const lineStarts = [];
  let offset = 0;
  for (const line of lines) {
    lineStarts.push(offset);
    offset += line.length + 1;
  }
  let labelMatch;
  while ((labelMatch = labelScan.exec(text)) !== null) {
    const at = labelMatch.index;
    if (captured.some(([start, end]) => at >= start && at < end)) continue;
    let lineIdx = lineStarts.findIndex(
      (start, i) => at >= start && (i + 1 >= lineStarts.length || at < lineStarts[i + 1]),
    );
    if (lineIdx === -1) continue;
    const collected = [];
    const lineMap = [];
    let nonJsonRun = 0;
    for (let i = lineIdx + 1; i < lines.length; i++) {
      const stripped = lines[i].trim().replace(/^`+|`+$/g, "").trim();
      if (!stripped) {
        if (collected.length > 0) break;
        nonJsonRun += 1;
        if (nonJsonRun > 2) break; // label was a prose mention, not a header
        continue;
      }
      if (stripped.startsWith("#")) break;
      if (!stripped.startsWith("{")) {
        if (collected.length > 0) break;
        nonJsonRun += 1;
        if (nonJsonRun > 2) break;
        continue;
      }
      collected.push(stripped);
      lineMap.push(i + 1); // 1-based absolute line in the quarantined file
    }
    if (collected.length > 0) {
      blocks.push({
        type: labelMatch[1].toLowerCase() === "verifier" ? "verifier" : "evidence",
        body: collected.join("\n"),
        label: `label-scan:${labelMatch[1].toLowerCase()}`,
        lineMap,
      });
    }
  }
  return blocks;
}

// Route a single parsed record to evidence vs verifier by its shape.
const VERIFIER_STATUS_ALIASES = new Map([
  ["pass", "passed"], ["passed", "passed"], ["ok", "passed"], ["green", "passed"], ["success", "passed"],
  ["fail", "failed"], ["failed", "failed"], ["red", "failed"], ["error", "failed"],
  ["skip", "skipped"], ["skipped", "skipped"],
  ["pending", "pending"], ["proposed", "proposed"], ["open", "pending"],
]);

function routeRecord(record, blockType) {
  if (blockType === "evidence" || blockType === "verifier") {
    // Explicit label wins unless the record is unmistakably the other kind.
    if (blockType === "evidence" && record.verifier_score === undefined && record.status === undefined) return "evidence";
    if (blockType === "verifier") return "verifier";
  }
  if (record.verifier_score !== undefined) return "verifier";
  const status = typeof record.status === "string" ? record.status.trim().toLowerCase() : null;
  if (status && VERIFIER_STATUS_ALIASES.has(status)) return "verifier";
  return "evidence";
}

// Conservative JSON repair ladder. Each variant must fully JSON.parse to be
// accepted; we never "half fix" content.
function repairJsonText(text) {
  const attempts = [];
  let current = text
    .replace(/[“”]/g, '"')
    .replace(/[‘’]/g, "'");
  attempts.push(current);
  // strip // and /* */ comments (crude but string-safe enough for records)
  attempts.push(current.replace(/^\s*\/\/.*$/gm, "").replace(/\/\*[\s\S]*?\*\//g, ""));
  // trailing commas
  const noTrailing = (s) => s.replace(/,\s*([}\]])/g, "$1");
  attempts.push(noTrailing(current));
  // quote unquoted keys
  const quotedKeys = (s) => s.replace(/([{,]\s*)([A-Za-z_][A-Za-z0-9_-]*)\s*:/g, '$1"$2":');
  attempts.push(noTrailing(quotedKeys(current)));
  // single-quoted strings -> double (only when it makes the whole thing parse)
  const singleToDouble = (s) =>
    s.replace(/'((?:[^'\\]|\\.)*)'/g, (whole, inner) => '"' + inner.replace(/"/g, '\\"') + '"');
  attempts.push(noTrailing(quotedKeys(singleToDouble(current))));
  for (const attempt of attempts) {
    try {
      return { value: JSON.parse(attempt), repaired: attempt !== text };
    } catch {
      // next attempt
    }
  }
  return null;
}

// Extract records from a block body: JSONL lines, pretty-printed multi-line
// objects (brace-balanced, string-aware), or a whole-block array - with the
// repair ladder applied at each level. Returns
// {records: [{value, lines:[startAbs,endAbs]|null}], notes, skipped}.
// Absolute 1-based line numbers come from block.baseLine (fenced blocks) or
// block.lineMap (label-scan blocks); null when neither is known.
function parseRecordsFromBlock(block) {
  const records = [];
  const notes = [];
  const skipped = [];
  const bodyLines = block.body.split(/\r?\n/);
  const absLine = (bodyIdx) => {
    if (Array.isArray(block.lineMap)) return block.lineMap[bodyIdx] ?? null;
    if (typeof block.baseLine === "number") return block.baseLine + bodyIdx;
    return null;
  };
  const push = (value, lines, note) => {
    const list = Array.isArray(value) ? value : [value];
    for (const item of list) {
      if (item && typeof item === "object" && !Array.isArray(item)) {
        records.push({ value: item, lines });
        if (note) notes.push(note);
      }
    }
  };

  const trimmedBody = block.body.trim();
  // Whole-block array (agents love emitting one JSON array).
  if (trimmedBody.startsWith("[")) {
    const whole = repairJsonText(trimmedBody);
    if (whole && Array.isArray(whole.value)) {
      const first = absLine(0);
      const last = absLine(bodyLines.length - 1);
      push(
        whole.value,
        first !== null && last !== null ? [first, last] : null,
        whole.repaired ? "repaired: whole-block array" : null,
      );
      return { records, notes, skipped };
    }
  }

  // Brace-balanced assembly: handles one-per-line JSONL AND pretty-printed
  // objects spanning many lines, in the same pass.
  let acc = "";
  let accStartIdx = 0;
  let depth = 0;
  let inString = false;
  let escaped = false;
  const flush = (endIdx) => {
    const chunk = acc.trim();
    acc = "";
    if (!chunk) return;
    const start = absLine(accStartIdx);
    const end = absLine(endIdx);
    const lines = start !== null && end !== null ? [start, end] : null;
    const parsed = repairJsonText(chunk);
    if (parsed) {
      push(parsed.value, lines, parsed.repaired ? `repaired: ${chunk.slice(0, 60)}...` : null);
    } else {
      skipped.push(chunk.slice(0, 120));
    }
  };
  for (let idx = 0; idx < bodyLines.length; idx++) {
    const line = bodyLines[idx];
    if (depth === 0 && !line.trim()) continue;
    if (depth === 0 && !line.trim().startsWith("{") && !acc) {
      // Non-JSON line between records (agents interleave commentary): skip it
      // silently rather than poisoning the accumulator.
      continue;
    }
    if (!acc) accStartIdx = idx;
    acc += (acc ? "\n" : "") + line;
    for (const ch of line) {
      if (escaped) { escaped = false; continue; }
      if (ch === "\\") { escaped = true; continue; }
      if (ch === '"') inString = !inString;
      if (inString) continue;
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
    }
    if (depth <= 0 && acc.trim()) {
      flush(idx);
      depth = 0;
      inString = false;
      escaped = false;
    }
  }
  if (acc.trim()) flush(bodyLines.length - 1);
  return { records, notes, skipped };
}

const ALLOWED_SOURCE_KINDS = new Set([
  "file",
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

const MAX_OBSERVED_AT_DRIFT_DAYS = 7;

function readRunManifest(runDir) {
  try {
    const manifestPath = path.join(runDir, "manifest.json");
    if (!fs.existsSync(manifestPath)) return { createdAt: null, repoRoot: null };
    const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
    return {
      createdAt: typeof manifest.created_at === "string" ? manifest.created_at : null,
      // F3: file citations resolve against the manifest's recorded project
      // root — never against this package's own directory.
      repoRoot: typeof manifest.repo_root === "string" && manifest.repo_root.length > 0
        ? manifest.repo_root
        : null,
    };
  } catch {
    return { createdAt: null, repoRoot: null };
  }
}

function validateObservedAt(recordId, observedAt, runCreatedAt) {
  if (!observedAt) return;
  const recordMs = Date.parse(observedAt);
  if (Number.isNaN(recordMs)) {
    throw new Error(`record ${recordId ?? "<unknown>"} observed_at "${observedAt}" is not a valid ISO8601 timestamp`);
  }
  if (!runCreatedAt) return;
  const anchorMs = Date.parse(runCreatedAt);
  if (Number.isNaN(anchorMs)) return;
  const driftDays = Math.abs(recordMs - anchorMs) / 86_400_000;
  if (driftDays > MAX_OBSERVED_AT_DRIFT_DAYS) {
    throw new Error(
      `record ${recordId ?? "<unknown>"} observed_at "${observedAt}" is outside the ${MAX_OBSERVED_AT_DRIFT_DAYS}-day window around run created_at "${runCreatedAt}"`,
    );
  }
}

function normalizeVerifierRecord(record) {
  const next = { ...record };
  if (typeof next.status !== "string") next.status = "pending";
  if (typeof next.verifier_score !== "number") next.verifier_score = 0;
  if (!["pending", "proposed", "passed", "failed", "skipped"].includes(next.status)) {
    throw new Error(
      `verifier finding ${record.id ?? "<unknown>"} status "${next.status}" is not in the allowed set (pending|proposed|passed|failed|skipped)`,
    );
  }
  // H6: accept `closure_reason` as an optional, typed field that records
  // why a "passed" finding is a bounded-audit / bounded-investigation closure
  // rather than a genuine green. Normalize whitespace; drop if empty.
  if (typeof next.closure_reason === "string") {
    const trimmed = next.closure_reason.trim();
    if (trimmed.length > 0) {
      next.closure_reason = trimmed;
    } else {
      delete next.closure_reason;
    }
  } else if (next.closure_reason !== undefined && next.closure_reason !== null) {
    throw new Error(
      `verifier finding ${record.id ?? "<unknown>"} closure_reason must be a string when present`,
    );
  } else if (next.closure_reason === null) {
    delete next.closure_reason;
  }
  return next;
}

// Field-synonym maps: agents abbreviate and rename under pressure. Aliases
// are format repair (free); they never touch trust semantics.
const SUMMARY_ALIASES = ["summary", "text", "note", "claim", "description", "finding", "detail", "message"];
const SOURCE_ALIASES = ["source_ids", "sources", "source", "source_id", "citations", "citation", "refs", "files", "file", "evidence_refs"];
const OBSERVED_ALIASES = ["observed_at", "observedAt", "timestamp", "time", "when", "date"];
const KIND_ALIASES = ["kind", "type", "category"];
const KNOWN_ID_PREFIXES = /^(file|command|test|log|raw|packet|verifier|evidence|objective|receipt):/;

// M1 trust boundary: only `receipts run` mints receipts. Agent-authored records
// claiming to BE receipts are downgraded to observations; agent citations of
// receipt ids are kept only when the id exists in the verified journal.
function loadReceiptIds(runDir) {
  try {
    const file = path.join(runDir, "receipts", "receipts.jsonl");
    if (!fs.existsSync(file)) return new Set();
    return new Set(
      fs
        .readFileSync(file, "utf8")
        .split(/\r?\n/)
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          try {
            return JSON.parse(line).id;
          } catch {
            return null;
          }
        })
        .filter(Boolean),
    );
  } catch {
    return new Set();
  }
}

function slugifyCitation(input) {
  return (
    String(input)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 64) || "citation"
  );
}

// Coerce one citation string toward a canonical source id. Handles bare
// path:line, "path line N", "path#L12", backticks, "file: path:12" spacing,
// missing line numbers, and free text (which becomes a label-hashed log: id -
// visible, traceable, never provenance).
function hasLineSuffix(pathish) {
  return /:(\d+)(?:-(\d+))?$/.test(pathish);
}

// NOTE: notes pushed into the second arg are FORMAT REPAIRS (free - they
// never demote a record); semantic problems (nonexistent paths, unminted
// receipts) are flagged separately as provenance warnings by the caller.
function coerceSourceId(rawInput, warnings) {
  let s = String(rawInput ?? "").trim().replace(/^[`'"]+|[`'"]+$/g, "");
  if (!s) return null;
  s = s.replace(/^(file|command|test|log|raw)\s*:\s*/i, (m, p) => `${p.toLowerCase()}:`);
  if (KNOWN_ID_PREFIXES.test(s)) {
    // file: ids without a :line suffix would starve ref synthesis downstream;
    // pin them to line 1 (whole-file citation intent).
    if (s.startsWith("file:") && !hasLineSuffix(s)) {
      const fixed = `${s.replace(/\\/g, "/")}:1`;
      warnings.push(`citation-coerced: "${rawInput}" -> "${fixed}" (missing line pinned to 1)`);
      return fixed;
    }
    return s;
  }
  const looksPathy = /[\\/]/.test(s) || /\.[a-z]{1,5}(:|#L|\s+line\s+|$)/i.test(s);
  if (looksPathy) {
    let candidate = s
      .replace(/#L(\d+)(?:-L?(\d+))?$/i, (m, a, b) => (b ? `:${a}-${b}` : `:${a}`))
      .replace(/\s+lines?\s+(\d+)(?:\s*[-–]\s*(\d+))?$/i, (m, a, b) => (b ? `:${a}-${b}` : `:${a}`))
      .replace(/\\/g, "/")
      .replace(/\s+/g, "");
    if (!hasLineSuffix(candidate)) candidate = `${candidate}:1`;
    const coerced = `file:${candidate}`;
    warnings.push(`citation-coerced: "${rawInput}" -> "${coerced}"`);
    return coerced;
  }
  const slug = `log:${slugifyCitation(s)}`;
  warnings.push(`citation-freeform: "${String(rawInput).slice(0, 80)}" -> ${slug} (not provenance)`);
  return slug;
}

// Normalize an arbitrary agent-emitted object into the canonical record shape
// BEFORE strict processing. Every transformation is recorded. Declared
// source_refs are deliberately DISCARDED (their ids are merged into
// source_ids): ingest synthesizes and hashes refs itself, so agent-authored
// refs carry zero information and were the single biggest rejection source in
// the field test.
function normalizeRecordShape(record, { blockType, laneSlug, index, observedAt, runDir, repoRoot, receiptIds }) {
  const warnings = Array.isArray(record.provenance_warnings) ? [...record.provenance_warnings] : [];
  const repairs = [];
  const next = { ...record };

  const pickAlias = (aliases, canonical) => {
    for (const key of aliases) {
      if (next[key] !== undefined && next[key] !== null && next[key] !== "") {
        if (key !== canonical) {
          next[canonical] = next[key];
          delete next[key];
          repairs.push(`aliased: ${key} -> ${canonical}`);
        }
        return true;
      }
    }
    return false;
  };

  pickAlias(SUMMARY_ALIASES, "summary");
  pickAlias(KIND_ALIASES, "kind");
  pickAlias(OBSERVED_ALIASES, "observed_at");
  pickAlias(SOURCE_ALIASES, "source_ids");

  if (typeof next.summary !== "string" || next.summary.trim() === "") {
    next.summary = JSON.stringify(record).slice(0, 200);
    repairs.push("defaulted: summary from record body");
  }

  // Merge declared source_refs' ids into source_ids, then drop the refs.
  if (Array.isArray(next.source_refs) && next.source_refs.length > 0) {
    const refIds = next.source_refs
      .map((ref) => (ref && typeof ref === "object" ? ref.source_id ?? ref.path : null))
      .filter(Boolean);
    next.source_ids = [...(Array.isArray(next.source_ids) ? next.source_ids : []), ...refIds];
    repairs.push("declared source_refs discarded (ingest synthesizes + hashes refs itself)");
  }
  delete next.source_refs;

  if (typeof next.source_ids === "string") next.source_ids = [next.source_ids];
  if (!Array.isArray(next.source_ids)) next.source_ids = [];
  const coerced = [];
  const seen = new Set();
  for (const id of next.source_ids) {
    // Citation coercion notes are format repairs (report-only), NOT
    // provenance warnings - a bare "src/x.ts:12" is an honest citation in
    // agents' native shorthand and must stay fact-eligible once backed.
    const out = coerceSourceId(id, repairs);
    if (out && !seen.has(out)) {
      seen.add(out);
      coerced.push(out);
    }
  }
  next.source_ids = coerced;

  // Downgrade file: citations that cannot resolve on disk BEFORE strict
  // processing would hard-fail the whole lane on them. The claim survives,
  // visibly unverifiable, and can never promote to a fact.
  next.source_ids = next.source_ids.map((id) => {
    const parsed = /^file:(.+?)(?::(\d+(?:-\d+)?))?$/.exec(id);
    if (!parsed) return id;
    const p = parsed[1];
    const resolved = resolveSourcePath(p, runDir, repoRoot);
    if (resolved && fs.existsSync(resolved)) return id;
    const downgraded = `log:unverifiable-${slugifyCitation(p)}`;
    warnings.push(`unverifiable-citation: ${id} (path not found under run dir or repo_root) -> ${downgraded}`);
    return downgraded;
  });

  // Receipt citations: keep only ids that exist in the runtime journal.
  // Claiming a receipt that was never minted is the oldest trick in the book.
  next.source_ids = next.source_ids.map((id) => {
    if (!id.startsWith("receipt:")) return id;
    const receiptId = id.slice("receipt:".length);
    if (receiptIds && receiptIds.has(receiptId)) return id;
    const downgraded = `log:unminted-receipt-${slugifyCitation(receiptId)}`;
    warnings.push(`unminted-receipt-claim: ${id} is not in receipts/receipts.jsonl -> ${downgraded}`);
    return downgraded;
  });

  // Receipt impersonation: only `receipts run` writes kind:"receipt".
  if (blockType !== "verifier" && String(next.kind ?? "").toLowerCase() === "receipt") {
    next.kind = "observation";
    warnings.push("receipt-impersonation: only receipts run mints receipt records; demoted to observation");
  }

  // Timestamps: invalid or missing -> ingest time plus a note, never a
  // rejection.
  if (typeof next.observed_at !== "string" || Number.isNaN(Date.parse(next.observed_at))) {
    if (next.observed_at !== undefined) repairs.push(`repaired: observed_at "${String(next.observed_at).slice(0, 40)}" -> ingest time`);
    next.observed_at = observedAt;
  }

  if (typeof next.id !== "string" || next.id.trim() === "") {
    const prefix = blockType === "verifier" ? "vf" : "ev";
    next.id = `${prefix}-auto-${laneSlug}-${index + 1}`;
    repairs.push(`defaulted: id -> ${next.id}`);
  }

  // Charset enforcement (worklist items and briefs later render ids/labels
  // near commands - a shell metacharacter in an id is an injection channel
  // into Prime). Format repair, not demotion.
  const sanitizeToken = (value, what) => {
    const clean = String(value).replace(/[^A-Za-z0-9:._/\\-]/g, "-");
    if (clean !== value) repairs.push(`sanitized ${what}: "${String(value).slice(0, 60)}" -> "${clean.slice(0, 60)}"`);
    return clean;
  };
  next.id = sanitizeToken(next.id, "id");
  next.source_ids = next.source_ids.map((id) => sanitizeToken(id, "source_id"));

  if (blockType === "verifier") {
    const status = typeof next.status === "string" ? next.status.trim().toLowerCase() : "";
    const mapped = VERIFIER_STATUS_ALIASES.get(status);
    if (mapped) {
      if (next.status !== mapped) repairs.push(`aliased: status "${next.status}" -> "${mapped}"`);
      next.status = mapped;
    } else if (next.status !== undefined && next.status !== null) {
      // Present-but-unrecognized status: park as "proposed" (non-passing, so
      // the gate surfaces it) rather than rejecting the record.
      repairs.push(`repaired: status "${String(next.status).slice(0, 30)}" -> "proposed"`);
      next.status = "proposed";
    }
    if (typeof next.verifier_score === "string") {
      const num = Number(next.verifier_score);
      if (Number.isFinite(num)) {
        next.verifier_score = num;
        repairs.push("repaired: verifier_score string -> number");
      }
    }
    delete next.kind; // verifier records carry finding_kind, not kind
  } else if (typeof next.kind !== "string" || next.kind.trim() === "") {
    next.kind = "observation";
    repairs.push("defaulted: kind -> observation");
  }

  if (warnings.length > 0) next.provenance_warnings = warnings;
  return { record: next, repairs };
}

// Zero-burden prose harvesting: extract claim-shaped lines from a lane's
// natural report. A line qualifies when it cites something that looks like a
// concrete file path (with an extension, optionally :line). Fenced blocks
// and headings are skipped; markdown noise is stripped; count is capped so a
// pathological lane can't flood the packet. Each harvested claim is marked
// rationale:"harvested-from-prose" so the report can show its origin.
// Two citation shapes: a path with a directory component (line optional), or
// a bare filename WITH a :line suffix - the line number signals citation
// intent and keeps ordinary prose mentions of e.g. package.json from
// harvesting as claims.
const PATHISH_CITATION = /(?:[A-Za-z0-9_.-]+[\/\\])+[A-Za-z0-9_.-]+\.[A-Za-z0-9]{1,6}(?::\d+(?:-\d+)?)?|[A-Za-z0-9_-]+\.[A-Za-z0-9]{1,6}:\d+(?:-\d+)?/g;
const HARVEST_CAP = 40;

function harvestProseClaims(text) {
  const claims = [];
  let inFence = false;
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (/^(```|~~~)/.test(line)) {
      inFence = !inFence;
      continue;
    }
    if (inFence || !line || line.startsWith("#")) continue;
    const citations = [...new Set([...line.matchAll(PATHISH_CITATION)].map((m) => m[0]))];
    if (citations.length === 0) continue;
    const summary = line
      .replace(/^[-*>|]+\s*/, "")
      .replace(/[`*_]/g, "")
      .trim()
      .slice(0, 300);
    if (summary.length < 12) continue;
    claims.push({
      value: {
        kind: "observation",
        summary,
        source_ids: citations,
        rationale: "harvested-from-prose",
      },
      lines: [i + 1, i + 1],
    });
    if (claims.length >= HARVEST_CAP) break;
  }
  return claims;
}

// Scan for a `BLOCKED <reason>` sentinel. Match is anchored to line starts
// and tolerates leading/trailing whitespace, so subagents can put it on the
// last line of an otherwise-prose response or inline in a report section.
function findBlockedSentinel(text) {
  const match = text.match(/^[ \t]*BLOCKED[ \t]+(.+?)[ \t]*$/m);
  if (!match) return null;
  const reason = match[1].trim();
  return reason.length > 0 ? reason : null;
}

// F4: attribution is a trust boundary, not a suggestion. The caller's
// --agent-id/--lane stamp ALWAYS wins; a record that declared a different
// identity for itself keeps it only as claimed_agent_id/claimed_lane. This is
// what stops one lane from impersonating three agents to beat the coverage
// floor or to dodge/manufacture contradictions.
function stampAttribution(record, { agentId, lane }) {
  const next = { ...record };
  if (typeof next.agent_id === "string" && next.agent_id !== "" && next.agent_id !== agentId) {
    next.claimed_agent_id = next.agent_id;
  }
  if (typeof next.lane === "string" && next.lane !== "" && next.lane !== lane) {
    next.claimed_lane = next.lane;
  }
  next.agent_id = agentId;
  next.lane = lane;
  return next;
}

function resolveSourcePath(sourcePath, runDir, repoRoot) {
  if (!sourcePath) return null;
  if (path.isAbsolute(sourcePath)) return sourcePath;
  if (runDir) {
    const insideRun = path.resolve(runDir, sourcePath);
    if (fs.existsSync(insideRun)) return insideRun;
  }
  // F3: project-tree citations resolve ONLY against manifest.repo_root.
  if (repoRoot) return path.resolve(repoRoot, sourcePath);
  return null;
}

function isDirectSourceId(sourceId) {
  return typeof sourceId === "string" && /^(file|command|test|log):/.test(sourceId);
}

// G1+G10: normalize a `file:<path>:<span>` source_id. When the path portion is
// absolute AND resolves to something inside `repoRoot`, rewrite it to
// `file:<repo-relative>:<span>` with forward slashes preserved. Absolute paths
// that resolve OUTSIDE the repo root are left as-is (still machine-specific,
// but at least honest). Non-file source_ids are returned unchanged.
const pathNormalizationLog = new Set();
function normalizeFileSourceId(sourceId, repoRoot) {
  if (typeof sourceId !== "string") return sourceId;
  // Only rewrite `file:...:line` forms. `command:`, `test:`, `log:` etc. pass
  // through untouched because they don't carry filesystem paths.
  const match = /^file:(.+):(\d+(?:-\d+)?)$/.exec(sourceId);
  if (!match) return sourceId;
  const rawPath = match[1];
  const span = match[2];
  // Repo-relative paths never need rewriting. `path.isAbsolute` detects
  // drive-letter (C:\, C:/) and POSIX-absolute forms on both platforms.
  if (!path.isAbsolute(rawPath)) return sourceId;
  // Some agents emit POSIX-absolute paths like `/c/Users/...`. Node's
  // `path.isAbsolute` returns true for those but `path.relative` treats them
  // as starting from filesystem root rather than the drive. Normalize to
  // forward slashes before comparing.
  let absolute;
  try {
    absolute = path.resolve(rawPath);
  } catch {
    return sourceId;
  }
  const normalizedAbs = absolute.replace(/\\/g, "/");
  const normalizedRoot = repoRoot.replace(/\\/g, "/");
  // Must strictly live inside repoRoot (not just share a prefix).
  const relative = path.relative(normalizedRoot, normalizedAbs);
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
    return sourceId;
  }
  const relForwardSlash = relative.replace(/\\/g, "/");
  const next = `file:${relForwardSlash}:${span}`;
  if (next !== sourceId) {
    const key = `${sourceId}->${next}`;
    if (!pathNormalizationLog.has(key)) {
      pathNormalizationLog.add(key);
      process.stderr.write(`ingest: normalized ${sourceId} -> ${next}\n`);
    }
  }
  return next;
}

// G1+G10: normalize a bare path string (as found in source_ref.path). Mirrors
// normalizeFileSourceId: absolute-and-inside-repo => repo-relative-forward-slash.
function normalizeFilePath(rawPath, repoRoot) {
  if (typeof rawPath !== "string" || rawPath.length === 0) return rawPath;
  if (!path.isAbsolute(rawPath)) return rawPath;
  let absolute;
  try {
    absolute = path.resolve(rawPath);
  } catch {
    return rawPath;
  }
  const normalizedAbs = absolute.replace(/\\/g, "/");
  const normalizedRoot = repoRoot.replace(/\\/g, "/");
  const relative = path.relative(normalizedRoot, normalizedAbs);
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
    return rawPath;
  }
  return relative.replace(/\\/g, "/");
}

// H1 kind allowlist: non-file source_refs whose hash is missing or not already a
// valid fnv1a-64 digest get auto-hashed from the source_id bytes. "raw" refs
// still hash from disk when the path resolves under the run dir — that branch
// is handled in normalizeSourceRefs itself.
const AUTO_HASH_KINDS = new Set(["command", "test", "log", "raw", "packet", "verifier", "evidence", "objective"]);

// H3 helper: parse a direct source_id of the form `file:<path>:<line>` /
// `command:<name>` / `test:<name>` / `log:<name>` into the fields a synthesized
// source_ref needs.
function parseDirectSourceId(sourceId) {
  if (typeof sourceId !== "string") return null;
  const fileMatch = /^file:(.+):(\d+(?:-\d+)?)$/.exec(sourceId);
  if (fileMatch) {
    return { kind: "file", path: fileMatch[1], span: fileMatch[2] };
  }
  const prefixMatch = /^(command|test|log):(.+)$/.exec(sourceId);
  if (prefixMatch) {
    return { kind: prefixMatch[1], path: prefixMatch[2], span: null };
  }
  return null;
}

function normalizeSourceRefs(record, observedAt, runCreatedAt, runDir, repoRoot) {
  // G1+G10: normalize every source_id and source_ref.path up front so the rest
  // of the pipeline (dedupe, contradiction detection, packet sources) sees the
  // same shape regardless of whether the agent cited an absolute or
  // repo-relative path. Mutate a shallow clone so the original record stays
  // untouched for diffability. Normalization is keyed on manifest.repo_root
  // (F3) — when the manifest has none, absolute paths pass through untouched
  // and the strict gate rejects them downstream.
  const provenanceWarnings = Array.isArray(record.provenance_warnings)
    ? [...record.provenance_warnings]
    : [];
  const normalizedRecord = { ...record };
  if (Array.isArray(normalizedRecord.source_ids) && repoRoot) {
    normalizedRecord.source_ids = normalizedRecord.source_ids.map((id) =>
      normalizeFileSourceId(id, repoRoot),
    );
  }
  if (Array.isArray(normalizedRecord.source_refs)) {
    normalizedRecord.source_refs = normalizedRecord.source_refs.map((ref) => {
      if (!ref || typeof ref !== "object") return ref;
      const next = { ...ref };
      if (typeof next.source_id === "string" && repoRoot) {
        next.source_id = normalizeFileSourceId(next.source_id, repoRoot);
      }
      // Only rewrite the path when we know this is a file-kind ref AND the
      // path is absolute-inside-repo. Raw/command/test/log paths keep their
      // declared form because they're not filesystem citations.
      if (next.kind === "file" && typeof next.path === "string" && repoRoot) {
        next.path = normalizeFilePath(next.path, repoRoot);
      }
      return next;
    });
  }

  const declaredRefs = Array.isArray(normalizedRecord.source_refs)
    ? normalizedRecord.source_refs
    : [];
  const declaredIds = new Set(
    declaredRefs
      .map((source) => (source && typeof source.source_id === "string" ? source.source_id : null))
      .filter(Boolean),
  );

  // H3: for every direct source_id the record declares, synthesize a
  // source_ref if the author didn't hand-write one. Downstream normalization
  // will fill in the hash (from disk for file/raw, from source_id bytes for
  // command/test/log).
  const synthesizedRefs = [];
  for (const sourceId of Array.isArray(normalizedRecord.source_ids)
    ? normalizedRecord.source_ids
    : []) {
    if (!isDirectSourceId(sourceId) || declaredIds.has(sourceId)) continue;
    const parsed = parseDirectSourceId(sourceId);
    if (!parsed) continue;
    synthesizedRefs.push({
      source_id: sourceId,
      path: parsed.path,
      kind: parsed.kind,
      hash: "placeholder-will-be-filled",
      hash_alg: "fnv1a-64",
      span: parsed.span,
      observed_at: normalizedRecord.observed_at ?? observedAt,
    });
    declaredIds.add(sourceId);
  }

  record = normalizedRecord;
  const sourceRefs = [...declaredRefs, ...synthesizedRefs];

  const normalizedRefs = sourceRefs.map((source) => {
    const next = { ...source };
    if (!next.kind) {
      throw new Error(`source_ref ${next.source_id ?? "<unknown>"} is missing kind`);
    }
    if (!ALLOWED_SOURCE_KINDS.has(next.kind)) {
      throw new Error(
        `source_ref ${next.source_id ?? "<unknown>"} kind "${next.kind}" is not in the allowed set (${[...ALLOWED_SOURCE_KINDS].join("|")})`,
      );
    }
    if (!next.observed_at) next.observed_at = observedAt;
    validateObservedAt(next.source_id, next.observed_at, runCreatedAt);
    if (!next.hash_alg) next.hash_alg = "fnv1a-64";
    if (next.hash_alg !== "fnv1a-64") {
      throw new Error(
        `source_ref ${next.source_id ?? "<unknown>"} hash_alg "${next.hash_alg}" is not supported (expected "fnv1a-64")`,
      );
    }
    if (next.kind === "file") {
      const resolved = resolveSourcePath(next.path, runDir, repoRoot);
      // Block self-referential state/ files by MECHANISM (path identity
      // against THIS run's state dir, not a layout-coupled regex): the
      // compiler regenerates <run-dir>/state/* on every recompile, so hashing
      // them at ingest guarantees a drift-mismatch on the next recurrence.
      if (resolved && runDir) {
        const stateDir = path.resolve(runDir, "state") + path.sep;
        if ((path.resolve(resolved) + path.sep).startsWith(stateDir)) {
          // H5: tell the agent what to cite instead, not just what not to.
          throw new Error(
            `source_ref ${next.source_id ?? "<unknown>"} points at compiler-generated state/ file (${next.path}); evidence must cite stable inputs — try the run's raw/ or worker-results/ files, or the original source code, not derived compiler outputs`,
          );
        }
      }
      if (!resolved || !fs.existsSync(resolved)) {
        throw new Error(
          `source_ref ${next.source_id ?? "<unknown>"} file path does not exist: ${next.path}` +
            (repoRoot
              ? ""
              : " (manifest.json has no repo_root — file citations cannot be resolved against the project tree)"),
        );
      }
      const bytes = fs.readFileSync(resolved);
      next.hash = fnv1aHash(bytes);
      next.hash_basis = "content";
      // H2: clip out-of-range spans on file refs to the actual line count
      // instead of hard-failing. Agents frequently guess end-of-range; auto-
      // clipping keeps the ingest contract useful without letting them point
      // at lines that do not exist.
      if (next.span) {
        const match = /^(\d+)(?:-(\d+))?$/.exec(String(next.span));
        if (match) {
          const parts = bytes.toString("utf8").split(/\r?\n/);
          if (parts.length > 0 && parts[parts.length - 1] === "") parts.pop();
          const lineCount = Math.max(1, parts.length);
          let start = Number(match[1]);
          let end = Number(match[2] ?? match[1]);
          const original = next.span;
          if (start < 1) start = 1;
          if (end < 1) end = 1;
          if (start > lineCount) start = lineCount;
          if (end > lineCount) end = lineCount;
          if (start > end) start = end;
          const clipped = start === end ? String(start) : `${start}-${end}`;
          if (clipped !== String(original)) {
            // F7: a repaired citation is a demoted citation. The clip is
            // recorded as a provenance warning on the record, and records
            // carrying warnings are never fact-eligible in the compiler.
            const warning = `span-clipped: ${next.source_id ?? "<unknown>"} "${original}" -> "${clipped}" (file has ${lineCount} line(s))`;
            provenanceWarnings.push(warning);
            process.stderr.write(`ingest: ${warning} — record demoted, not fact-eligible\n`);
            next.span = clipped;
          }
        }
      }
    } else if (next.kind === "raw") {
      // raw refs: if the path resolves under the run dir, prefer the on-disk
      // hash (existing behavior). Otherwise fall through to H1 auto-hashing.
      const rawCandidate = runDir && next.path
        ? (path.isAbsolute(next.path) ? next.path : path.resolve(runDir, next.path))
        : null;
      if (rawCandidate && fs.existsSync(rawCandidate)) {
        next.hash = fnv1aHash(fs.readFileSync(rawCandidate));
        next.hash_basis = "content";
      } else if (!/^[0-9a-f]{16}$/.test(String(next.hash ?? ""))) {
        // H1 fallback for raw refs whose path didn't resolve on disk.
        next.hash = fnv1aHash(Buffer.from(String(next.source_id ?? ""), "utf8"));
        next.hash_basis = "label";
      }
    } else if (AUTO_HASH_KINDS.has(next.kind)) {
      // H1 + F2 truth-in-labeling: command/test/log/etc. have no on-disk
      // artifact yet (receipts land in M2), so the digest is derived from the
      // source_id STRING. That is an identity key, not provenance — it is
      // stamped hash_basis:"label" and never counts as a direct anchor.
      if (!/^[0-9a-f]{16}$/.test(String(next.hash ?? ""))) {
        next.hash = fnv1aHash(Buffer.from(String(next.source_id ?? ""), "utf8"));
      }
      next.hash_basis = "label";
    }
    return next;
  });

  const sourceIds = new Set(Array.isArray(record.source_ids) ? record.source_ids : []);
  for (const source of normalizedRefs) {
    if (!source.source_id) throw new Error(`record ${record.id ?? "<unknown>"} has source_ref without source_id`);
    sourceIds.add(source.source_id);
  }
  const normalizedRefIds = new Set(normalizedRefs.map((source) => source.source_id));
  for (const sourceId of sourceIds) {
    if (isDirectSourceId(sourceId) && !normalizedRefIds.has(sourceId)) {
      // H3 should prevent this branch for parseable direct ids, but keep the
      // throw for defensive coverage (e.g. file:something with no colon+line).
      throw new Error(
        `record ${record.id ?? "<unknown>"} lists direct source_id ${sourceId} without matching source_refs entry`,
      );
    }
  }

  return {
    ...record,
    observed_at: record.observed_at ?? observedAt,
    source_ids: [...sourceIds],
    ...(normalizedRefs.length > 0 ? { source_refs: normalizedRefs } : {}),
    ...(provenanceWarnings.length > 0 ? { provenance_warnings: provenanceWarnings } : {}),
  };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (!fs.existsSync(args.runDir)) fail(`run dir does not exist: ${args.runDir}`);

  const rawText = readInput(args);
  if (!rawText.trim()) fail("subagent output is empty");

  const stamp = utcStamp();
  const observedAt = new Date().toISOString();
  const { createdAt: runCreatedAt, repoRoot } = readRunManifest(args.runDir);
  const receiptIds = loadReceiptIds(args.runDir);
  const rawDir = path.join(args.runDir, "raw", "subagents");
  fs.mkdirSync(rawDir, { recursive: true });
  const directRaw = isInsideDir(args.from, rawDir);
  const rawPath = directRaw ? args.from : path.join(rawDir, `${stamp}-${slugify(args.lane)}-${slugify(args.agentId)}.md`);
  if (!directRaw) {
    fs.writeFileSync(
      rawPath,
      [
        `# Subagent Session ${stamp}`,
        "",
        `lane: ${args.lane}`,
        `agent_id: ${args.agentId}`,
        "",
        rawText.trim(),
        "",
      ].join("\n"),
      "utf8",
    );
  }

  const rawName = normalizeRawName(rawDir, rawPath);
  const rawSourceId = `raw:subagents/${rawName}`;
  // Phase 3: all extraction runs against the FINAL quarantined bytes (never
  // the pre-quarantine input - the header rewrite shifted line coordinates,
  // review finding 9), so drill-down spans are exact and the span refs hash
  // the same file the gate re-verifies.
  const quarantinedBuffer = fs.readFileSync(rawPath);
  const quarantinedText = quarantinedBuffer.toString("utf8");
  const quarantinedHash = fnv1aHash(quarantinedBuffer);
  const runRelativeRawPath = path.relative(args.runDir, rawPath).replace(/\\/g, "/");

  // G9: refuse to append a second subagent-session record for the same raw
  // file. Retrying ingest against the same --from already-quarantined file is
  // almost always an error — it double-counts the session and double-stamps
  // raw:subagents/... source_ids. Detect BEFORE block parsing so the fail path
  // is deterministic regardless of whether the raw file has fenced records.
  const existingEvidencePath = path.join(args.runDir, "worker-results", "evidence.jsonl");
  if (fs.existsSync(existingEvidencePath)) {
    const existingLines = fs.readFileSync(existingEvidencePath, "utf8").split(/\r?\n/);
    for (const line of existingLines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      let parsed;
      try {
        parsed = JSON.parse(trimmed);
      } catch {
        continue;
      }
      if (parsed && parsed.kind === "subagent-session") {
        const sourceIds = Array.isArray(parsed.source_ids) ? parsed.source_ids : [];
        if (sourceIds.includes(rawSourceId)) {
          fail(
            `duplicate ingest: raw/subagents/${rawName} already has a subagent-session record (${parsed.id ?? "<unknown>"}); remove the prior record or use a new --from path`,
          );
        }
      }
    }
  }

  const blocks = parseBlocks(quarantinedText);
  const blockedReason = findBlockedSentinel(quarantinedText);

  const evidence = [];
  const findings = [];
  const repairNotes = [];
  const skippedRecords = [];
  const laneSlug = slugify(args.lane);
  let recordIndex = 0;

  const processOne = (record, routed, lines) => {
    validateObservedAt(record.id, record.observed_at, runCreatedAt);
    const normalized = normalizeSourceRefs(record, observedAt, runCreatedAt, args.runDir, repoRoot);
    const stamped = stampAttribution(normalized, { agentId: args.agentId, lane: args.lane });
    // H4 + Phase 3: every record carries its quarantined-raw provenance. When
    // the extractor knows WHERE in the raw file the record came from, the id
    // is span-suffixed (raw:subagents/<file>:<start>-<end>) with a matching
    // hash-bearing source_ref - Prime's drill-down handle.
    const spanId =
      Array.isArray(lines) && lines.length === 2
        ? `${rawSourceId}:${lines[0]}-${lines[1]}`
        : rawSourceId;
    const spanRef =
      spanId === rawSourceId
        ? null
        : {
            source_id: spanId,
            path: runRelativeRawPath,
            kind: "raw",
            hash: quarantinedHash,
            hash_alg: "fnv1a-64",
            hash_basis: "content",
            span: `${lines[0]}-${lines[1]}`,
            observed_at: observedAt,
          };
    const withRaw = {
      ...stamped,
      source_ids: (stamped.source_ids ?? []).includes(spanId)
        ? stamped.source_ids
        : [spanId, ...(stamped.source_ids ?? [])],
      ...(spanRef
        ? { source_refs: [...(stamped.source_refs ?? []), spanRef] }
        : {}),
    };
    return routed === "verifier" ? normalizeVerifierRecord(withRaw) : withRaw;
  };

  for (const block of blocks) {
    const parsed = parseRecordsFromBlock(block);
    repairNotes.push(...parsed.notes);
    for (const chunk of parsed.skipped) {
      skippedRecords.push({ reason: "unparseable-json", preview: chunk });
    }
    for (const raw of parsed.records) {
      const routed = routeRecord(raw.value, block.type);
      const shaped = normalizeRecordShape(raw.value, {
        blockType: routed,
        laneSlug,
        index: recordIndex,
        observedAt,
        runDir: args.runDir,
        repoRoot,
        receiptIds,
      });
      recordIndex += 1;
      repairNotes.push(...shaped.repairs.map((note) => `${shaped.record.id}: ${note}`));
      let finalRecord;
      try {
        finalRecord = processOne(shaped.record, routed, raw.lines);
      } catch (error) {
        // Last-resort salvage: reset the timestamp (drift-window rejections)
        // and retry once. A record that still fails is skipped WITH a reason
        // in the report - it never takes the whole lane down with it.
        try {
          finalRecord = processOne(
            {
              ...shaped.record,
              observed_at: observedAt,
              provenance_warnings: [
                ...(shaped.record.provenance_warnings ?? []),
                `salvaged: ${String(error.message).slice(0, 140)}`,
              ],
            },
            routed,
            raw.lines,
          );
        } catch (error2) {
          skippedRecords.push({
            id: shaped.record.id ?? "<unknown>",
            reason: String(error2.message).slice(0, 200),
          });
          continue;
        }
      }
      if (routed === "verifier") findings.push(finalRecord);
      else evidence.push(finalRecord);
    }
  }

  // ZERO-BURDEN CAPTURE (John's design constraint, 2026-07-13: "I can't
  // change agent behaviour, I can only give receipts to Prime"). When a lane
  // returned no machine records, the ENGINE does the structuring: harvest
  // claim-shaped lines from the natural prose - any sentence citing a
  // concrete file path becomes its own asserted-tier evidence record with
  // coerced, hash-verified citations. Extraction can never create trust:
  // harvested claims promote only if receipts or verifiers later back them.
  let harvested = 0;
  if (evidence.length === 0 && findings.length === 0 && !blockedReason) {
    const proseClaims = harvestProseClaims(quarantinedText);
    for (const raw of proseClaims) {
      const shaped = normalizeRecordShape(raw.value, {
        blockType: "evidence",
        laneSlug,
        index: recordIndex,
        observedAt,
        runDir: args.runDir,
        repoRoot,
        receiptIds,
      });
      recordIndex += 1;
      try {
        evidence.push(processOne(shaped.record, "evidence", raw.lines));
        harvested += 1;
      } catch {
        // a single unharvestable line is not worth a report entry
      }
    }
    if (harvested > 0) {
      repairNotes.push(`harvested ${harvested} claim(s) from natural prose (no machine records in lane output)`);
    }
  }

  // Prose-only lane with nothing harvestable: capture, don't crash. The
  // packet shows one demoted "unstructured" record pointing at the raw file,
  // so the orchestrator sees a degraded lane instead of a dead pipeline. It
  // can never become a fact and never counts toward agent coverage.
  let unstructured = false;
  if (evidence.length === 0 && findings.length === 0 && !blockedReason) {
    unstructured = true;
    const firstProse = rawText
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((line) => line && !line.startsWith("#") && !line.startsWith("```"))
      .join(" ")
      .slice(0, 280);
    evidence.push(
      stampAttribution(
        {
          id: `ev-unstructured-${stamp}-${laneSlug}`,
          kind: "unstructured",
          summary: `Lane returned prose without machine records: ${firstProse || "(empty prose)"}`,
          source_ids: [rawSourceId],
          observed_at: observedAt,
          provenance_warnings: [
            "unstructured: no machine-readable records found; content quarantined only - claims in this lane are unverified narrative",
          ],
        },
        { agentId: args.agentId, lane: args.lane },
      ),
    );
  }

  // Blocker synthesis: any subagent that signals BLOCKED emits a machine-
  // readable blocker record, whether or not it also supplied fenced evidence.
  // A partial success can still report "this lane is blocked on X" without
  // discarding the evidence it did gather.
  if (blockedReason) {
    evidence.push(
      stampAttribution(
        {
          id: `ev-blocker-${stamp}-${slugify(args.lane)}`,
          kind: "blocker",
          summary: `BLOCKED: ${blockedReason}`,
          source_ids: [rawSourceId],
          observed_at: observedAt,
        },
        { agentId: args.agentId, lane: args.lane },
      ),
    );
  }

  evidence.unshift({
    id: `ev-subagent-session-${stamp}-${slugify(args.lane)}`,
    kind: "subagent-session",
    summary: `Captured quarantined subagent output for lane ${args.lane}; machine records were extracted without Prime synthesis.`,
    source_ids: [rawSourceId],
    source_refs: [
      {
        source_id: rawSourceId,
        path: runRelativeRawPath,
        kind: "raw",
        hash: fnv1aHash(fs.readFileSync(rawPath)),
        hash_alg: "fnv1a-64",
        span: null,
        observed_at: observedAt,
      },
    ],
    observed_at: observedAt,
    agent_id: args.agentId,
    lane: args.lane,
  });

  appendJsonl(path.join(args.runDir, "worker-results", "evidence.jsonl"), evidence);
  appendJsonl(path.join(args.runDir, "verifier-results", "findings.jsonl"), findings);

  process.stdout.write(
    `${JSON.stringify(
      {
        ok: true,
        raw_path: rawPath,
        raw_source_id: rawSourceId,
        evidence_records: evidence.length,
        verifier_records: findings.length,
        unstructured,
        harvested,
        repairs: repairNotes.length,
        repair_notes: repairNotes.slice(0, 20),
        skipped_records: skippedRecords.slice(0, 20),
      },
      null,
      2,
    )}\n`,
  );
}

try {
  main();
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}
