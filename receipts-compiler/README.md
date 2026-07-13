# receipts-skill

Deterministic packet compiler for AI agent runs.

Takes raw subagent output (evidence JSONL, verifier findings, raw artifacts) and compiles a schema-validated, hash-provenanced `next_pass_packet.json` that the orchestrating model reads instead of raw subagent prose.

Part of the [receipts-skill](https://github.com/inchwormz/receipts-skill) project — see the repo for the full JS runtime (ingest, strict gate, readiness) that drives this crate.

## What this crate does

- Reads a run directory containing `manifest.json`, `worker-results/evidence.jsonl`, `verifier-results/findings.jsonl`, and `raw/` artifacts.
- Validates every `source_ref` — hashes `file:` references against disk, checks line-range spans, enforces provenance rules for substantive evidence kinds.
- Promotes evidence into `trusted_facts` using agent-supplied `confidence` when present.
- Auto-detects `Contradiction` entries when different agents assert divergent summaries on the same direct source span, graduating severity by evidence kind.
- Emits `next_pass_packet.json`, `snapshot.json`, `decision_log.jsonl` — byte-deterministic for byte-identical inputs.

## Install

```bash
cargo install receipts-skill
```

## Run

```bash
receipts compile --run-dir <run-dir>
```

Expected run directory shape:

- `manifest.json`
- `task.md`
- `raw/`
- `worker-results/evidence.jsonl`
- `verifier-results/findings.jsonl`

Outputs land in `state/` inside the run directory.

## Minimal evidence shape

```json
{
  "id": "ev-example",
  "kind": "code-change",
  "summary": "The timeout helper is now used by Firecrawl API calls.",
  "agent_id": "receipts-evidence-worker",
  "lane": "impl",
  "confidence": 0.9,
  "rationale": "Read at file:scripts/foo.js:42.",
  "source_ids": ["file:skills/foo.js:42"],
  "source_refs": [
    {
      "source_id": "file:skills/foo.js:42",
      "path": "skills/foo.js",
      "kind": "file",
      "hash": "<fnv1a-64>",
      "hash_alg": "fnv1a-64",
      "span": "42",
      "observed_at": "2026-04-21T00:00:00Z"
    }
  ],
  "observed_at": "2026-04-21T00:00:00Z"
}
```

## Determinism guarantee

The `compile_determinism` integration test runs the compiler twice on a byte-identical fixture and asserts byte-identical `next_pass_packet.json` + `snapshot.json`. A regression here means a non-deterministic code path slipped in.

## License

MIT
