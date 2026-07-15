# Receipts Prove Command Implementation Plan

> **For Codex:** Execute this plan task-by-task with fresh red/green evidence at every boundary.

**Goal:** Replace the easy-to-misuse manual `init -> absorb -> check -> compile -> conclude` path with one fail-closed `receipts prove` command, while preserving the granular commands for advanced use.

**Product mechanism:** `prove` initializes or resumes a run, absorbs every explicitly attributed report, selects every repo-declared check unless the operator explicitly narrows it, runs those checks through the existing subject-binding engine, then synthesizes, gates, reports, and returns nonzero if any stage fails. The command never invents checks or turns raw command success into an attested fact.

**Progress measure:** happy-path operator decisions fall from at least five commands to one. Done means the new command has a deliberate red witness, a green end-to-end test, the official manifest-bound Node check passes, the isolated Rust suite passes, and `receipts prove` produces a real signed check attempt.

## Stage 1: Repair the live check signal

**Files:** `.receipts/checks.toml`

1. Preserve the current failed `node-suite` receipt and its two isolated green witnesses.
2. Falsify a shared-target publication race with isolated and six-way concurrent identity runs.
3. Because those witnesses pass, treat whole-suite process oversubscription as the measurement defect and serialize the official Node check.
4. Add a real Node test-runner negative control to the manifest and bump its definition version.
5. Re-run the manifest-bound `node-suite` check.

## Stage 2: Specify `prove` red-first

**Files:** `tests/loop_composites.test.js`, `tests/rust_engine_commands.test.js`, `bin/receipts.mjs`

1. Add a fixture repo with two cheap declared checks and an attributed lane report.
2. Assert one command initializes the run, absorbs the report, runs both checks by default, concludes, writes the HTML report, and returns zero.
3. Assert a failing declared check still leaves its signed attempt and report but makes `prove` return nonzero.
4. Assert `prove` rejects malformed report specs and missing check manifests.
5. Assert `prove` rejects a superficially green result when zero report claims are actually bound to checks.
6. Run only the new tests and capture the expected unknown-command failures.

## Stage 3: Implement the chain in the Rust authority

**Files:** `receipts-compiler/src/bin/receipts_core.rs`, `bin/receipts.mjs`

1. Add `prove` to both dispatchers and help output.
2. Parse repeatable `--report <lane>:<agent-id>:<path>` and optional repeatable `--check <id>` flags.
3. Initialize a missing run with `--repo-root` or the current directory; otherwise validate and resume it.
4. Absorb reports using the existing composite, load checks only from `.receipts/checks.toml`, run all declared checks by default, and continue long enough to emit the final gate/report.
5. Return nonzero if any report, check, or gate fails, or if zero claims are check-bound; print a concise machine-readable stage summary plus the existing human brief.

## Stage 4: Make the safe path obvious

**Files:** `README.md`, `SKILL.md`, `docs/HOW-IT-WORKS.md` if applicable

1. Put `receipts prove` first in the quick start.
2. Label `run` as low-level execution evidence that does not attest claims.
3. Explain that checks are repo-owned and all run by default, so the agent does not decide what counts as proof.

## Stage 5: Fresh verification and handoff

1. Run the new focused Node tests after the final edit.
2. Run the official `node-suite` through `receipts check` and inspect its negative-control fields.
3. Run the Rust suite with a fresh isolated `CARGO_TARGET_DIR`.
4. Run one real `receipts prove` canary and inspect attempts, receipts, gate report, and HTML report.
5. Record exact outputs and remaining limitations in `RECEIPTS-RELIABILITY-LEDGER.md` and the global session handoff.
