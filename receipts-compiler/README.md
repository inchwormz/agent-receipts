# Agent Receipts engine

Deterministic packet compiler for AI agent runs.

Takes raw subagent output (evidence JSONL, verifier findings, raw artifacts) and compiles a schema-validated, hash-provenanced `next_pass_packet.json` that the orchestrating model reads instead of raw subagent prose.

Part of the [agent-receipts](https://github.com/inchwormz/agent-receipts) project — see the repo for the full runtime that drives this crate.

## What this crate does

- Reads a run directory containing `manifest.json`, `worker-results/evidence.jsonl`, `verifier-results/findings.jsonl`, and `raw/` artifacts.
- Validates every `source_ref` — hashes `file:` references against disk, checks line-range spans, enforces provenance rules for substantive evidence kinds.
- Emits schema `2.0.0` typed trust: integrity, command outcome, applicability, and claim status are independent engine-owned fields.
- Promotes a claim only through a current `.receipts/checks.toml` binding over exact subject bytes, dependency locks, environment, check version, and target claim. Agent confidence is retained only as `reported_confidence` diagnostics.
- Auto-detects `Contradiction` entries when different agents assert divergent summaries on the same direct source span, graduating severity by evidence kind.
- Emits `next_pass_packet.json`, `snapshot.json`, `decision_log.jsonl` — byte-deterministic for byte-identical inputs.

## Install

Install the standalone Rust CLI from crates.io:

```bash
cargo install receipts-core --locked
receipts doctor
```

Or install the full npm dispatcher plus bundled engine source from the public
repository with `npm install -g github:inchwormz/agent-receipts`.

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
  "reported_confidence": 0.9,
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
