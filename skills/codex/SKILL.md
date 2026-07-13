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

Why it exists: agents summarize, embellish, and occasionally lie — and you cannot change agent behavior. Receipts never asks them to. It attests what actually ran (hash-chained execution receipts), structures whatever the lanes wrote (however messy), and promotes ONLY runtime-backed claims to trusted facts. Everything else stays labeled as the hearsay it is.

## The loop (3 motions)

```bash
# 0. Once per task: scaffold a run (default root: .receipts/runs/)
receipts init .receipts/runs/<task-slug>

# 1. Per lane, after it reports: save the lane's VERBATIM text to a file, then
receipts absorb --run-dir <d> --lane <lane> --agent-id <id> --from <report.md>
#    -> ingests (any format), mints a WORK receipt of tree changes, recompiles.

# 2. Never trust a lane's "tests pass" — mint the proof yourself:
receipts run --run-dir <d> --label test:<name> -- <the actual command>
#    A claim citing test:<name> becomes ATTESTED if the receipt passed,
#    and is mechanically REFUTED (gate red) if it failed.

# 3. Close the pass:
receipts conclude --run-dir <d> --synthesis "<what happened this pass>"
#    -> recompiles, gates, writes state/report.html, prints the Prime brief.
#    Its exit code IS the gate: 0 = green, nonzero = read the brief's worklist.
```

Blocking worklist item you have adjudicated? Clear it on the record:

```bash
receipts resolve --run-dir <d> --target <worklist-or-contradiction-id> --reason "…" [--cite <source>]
```

## Doctrine (what makes the receipts worth anything)

- **Zero-burden lanes.** Briefs are TASK-ONLY — describe the work, never the reporting format. NEVER re-prompt a lane about format; ingest structures prose, broken JSON, shorthand, anything (claims harvested from natural sentences get hash-verified citations automatically). Escalating format demands at lanes is the named failure that nearly killed this product.
- **Receipts are minted only by the runtime.** `receipts run` and absorb's work diff are the only mints. Agent text claiming receipt ids it does not own is demoted automatically. Never edit `receipts/`, `decisions/`, or `state/` by hand — tampering is a fatal compile error, by design.
- **The trust ladder is the report.** attested (receipt-backed) > verifier > asserted (unproven claim) > narrative (indexed prose, never trusted). Repeat claims to others at their proven tier, not their told tier.
- **A passing label attests one thing** — "this exact command exited 0". Mint the label a claim actually cites; a green `test:a` proves nothing about `test:b`.
- **Capture exit codes honestly.** Pipes swallow them (`cmd | tail` reports tail's exit). Conclude's own exit is the gate verdict — read it directly, never through a pipe.
- **Done means gate green.** A run is not done because code changed or a lane said tests passed. It is done when `receipts conclude` exits 0 and the brief's worklist is clear.

## All commands

| Command | What it does |
| --- | --- |
| `receipts init <dir>` | scaffold a run directory |
| `receipts absorb --run-dir d --lane l --agent-id a --from f` | ingest + work receipt + recompile (one motion per lane) |
| `receipts run --run-dir d --label test:x -- cmd…` | execute + mint a tamper-evident execution receipt |
| `receipts conclude --run-dir d --synthesis "…"` | recompile + gate + report + brief; exit = gate |
| `receipts resolve --run-dir d --target id --reason "…"` | hash-chained adjudication clearing a blocking item |
| `receipts diff --run-dir d [--note t]` | standalone WORK receipt of tree changes |
| `receipts next --run-dir d [--json]` | reprint the compressed Prime brief |
| `receipts ingest / compile / gate --run-dir d …` | the loop's individual gears, when you need just one |
| `receipts ready` | end-to-end pipeline self-check |

## Runtime boundary

Codex is Prime. The local runtime (Rust `receipts-core` + the Node `receipts` CLI) is the body. Spawn lanes with Codex's own subagent mechanism; Prime reads the compiled brief and drill-down spans — never raw subagent chat as ground truth. This is a Codex skill, not a Claude batch runner — do not spawn Claude sessions from within it.

## Troubleshooting

- Engine identity failure → reinstall from `github:inchwormz/agent-receipts`; the package rebuilds its bundled Rust source and verifies protocol, commit, lockfile, platform, and digest before execution.
- Legacy runs live under `.mythos/` (the old product name) — still readable with explicit `--run-dir`; new runs go under `.receipts/`. Legacy ```mythos-evidence-jsonl fences are accepted forever.
- Gate red you believe is wrong → suspect the instrument first: is the packet fresh (`receipts compile`)? Is the binary current? The gate's errors name exact record ids — drill down before overriding.
