---
name: receipts
description: Proof-of-work for AI agent claims. Mint tamper-evident execution receipts, absorb natural-prose lane reports with zero format burden, and gate to receipt-backed facts you can hand to anyone.
keywords:
  - receipts
  - proof-of-work
  - attestation
  - subagent verification
  - provenance
---

# Receipts — Codex skill

Use this skill when you orchestrate subagents, or run any work whose claims someone must trust later. You are **Prime**: the agent who has to vouch for what happened. Receipts gives you facts proven by the runtime, not narrated by agents.

Why it exists: agents summarize, embellish, and occasionally lie — and you cannot change agent behavior. Receipts never asks them to. It records what ran as hash-chained events, structures whatever the lanes wrote, and promotes only claims with a current engine-owned check binding. Everything else stays asserted.

## The loop (one safe motion)

```bash
receipts prove --run-dir .receipts/runs/<task-slug> \
  --report <lane>:<agent-id>:<report.md> \
  --synthesis "<what happened this pass>"
# Repeat --report for every lane. Prove initializes or resumes, absorbs reports,
# runs ALL checks from .receipts/checks.toml by default, concludes, reports, and
# fails if any stage is red or zero report claims are actually check-bound.
```

Do not choose checks manually on the normal path. Repeated `--check <id>` is an
advanced narrowing control only. Use the granular commands below to diagnose a
red run, not as the default orchestration recipe.

Blocking worklist item you have adjudicated? Clear it on the record:

```bash
receipts resolve --run-dir <d> --target <worklist-or-contradiction-id> --reason "…" [--cite <source>]
```

## Doctrine (what makes the receipts worth anything)

- **Zero-burden lanes.** Briefs are TASK-ONLY — describe the work, never the reporting format. NEVER re-prompt a lane about format; ingest structures prose, broken JSON, shorthand, anything (claims harvested from natural sentences get hash-verified citations automatically). Escalating format demands at lanes is the named failure that nearly killed this product.
- **Receipts are minted only by the runtime.** `receipts run` and absorb's work diff are the only mints. Agent text claiming receipt ids it does not own is demoted automatically. Never edit `receipts/`, `decisions/`, or `state/` by hand — tampering is a fatal compile error, by design.
- **Trust is typed, not a ladder.** Integrity, command outcome, applicability, and claim status are separate. Agent confidence is `reported_confidence` only.
- **A passing label proves no semantic claim.** Use `receipts check`; a relevant subject, lock, environment, or check-version change automatically makes the result stale.
- **Capture exit codes honestly.** Pipes swallow them (`cmd | tail` reports tail's exit). Conclude's own exit is the gate verdict — read it directly, never through a pipe.
- **Done means prove green with bound facts.** A run is not done because code changed, checks merely executed, or a lane said tests passed. It is done when `receipts prove` exits 0, reports `bound_claims > 0`, and the brief's worklist is clear.

## All commands

| Command | What it does |
| --- | --- |
| `receipts prove --run-dir d --report lane:agent:file --synthesis "…"` | safe default: init/resume + absorb + all checks + gate + report + brief |
| `receipts init <dir>` | scaffold a run directory |
| `receipts absorb --run-dir d --lane l --agent-id a --from f` | ingest + work receipt + recompile (one motion per lane) |
| `receipts run --run-dir d --label test:x -- cmd…` | low-level execution event only; never verifies a semantic claim |
| `receipts check --run-dir d --id x` | execute a declared check and bind it to subject, environment, and target claims |
| `receipts conclude --run-dir d --synthesis "…"` | recompile + gate + report + brief; exit = gate |
| `receipts resolve --run-dir d --target id --reason "…"` | hash-chained adjudication clearing a blocking item |
| `receipts diff --run-dir d [--note t]` | standalone WORK receipt of tree changes |
| `receipts next --run-dir d [--json]` | reprint the compressed Prime brief |
| `receipts ingest / compile / gate --run-dir d …` | the loop's individual gears, when you need just one |
| `receipts ready` | end-to-end pipeline self-check |

## Runtime boundary

Codex is Prime. The local runtime is the Rust `receipts` engine; npm supplies only its dispatcher. Spawn lanes with Codex's own subagent mechanism; Prime reads the compiled brief and drill-down spans — never raw subagent chat as ground truth. This is a Codex skill, not a Claude batch runner — do not spawn Claude sessions from within it.

## Troubleshooting

- Engine identity failure → reinstall from `github:inchwormz/agent-receipts`; the package rebuilds its bundled Rust source and verifies protocol, commit, lockfile, platform, and digest before execution.
- Legacy runs live under `.mythos/` (the old product name) — still readable with explicit `--run-dir`; new runs go under `.receipts/`. Legacy ```mythos-evidence-jsonl fences are accepted forever.
- Gate red you believe is wrong → suspect the instrument first: is the packet fresh (`receipts compile`)? Is the binary current? The gate's errors name exact record ids — drill down before overriding.
