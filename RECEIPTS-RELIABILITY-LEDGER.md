# Agent Receipts Reliability Ledger

Last updated: 2026-07-14 (Pacific/Auckland)

## Goal

Build the local-first Receipts reliability system in this canonical repository, with typed trust, cryptographic engine identity, independently adjudicated outcomes, calibrated false-green risk, deterministic public cards, and a gated reliability index.

## Canonical boundary

- Writable canonical repository: `C:\Users\johnr\Projects\agent-receipts`
- Read-only migration input: `C:\Users\johnr\Projects\mythos-skill`
- Excluded migration content: site WIP, `dist`, generated output, and Playwright files
- Initial public HEAD: `36b517d`
- Initial branch: `main`
- Initial tree: clean
- Push, package publication, GitHub Pages, and edits to `mythos-skill`: forbidden without separate instruction

## Campaign measures

- Trust: **0/3 demonstrated false-green mechanisms blocked at baseline; target 3/3 before statistics begin.**
- Calibration: **0 independently adjudicated outcomes at baseline; no probability may be presented before eligibility.**
- Product headline: **upper 95% bound of false-green risk**, with verified completion separate.

## Stage map

1. **Canonicalize baseline and engine launch**
   - Output: four migrated mechanisms with red/green receipts, canonical URLs, explicit binary identity handshake, clean full-suite result.
   - Done: public checkout installs alone; incompatible binary is rejected; source stays clean after verification.
2. **Typed trust (`2.0.0`)**
   - Output: integrity, outcome, applicability, claim status, Evidence Coverage, bound check manifest, negative controls, full retry history, authenticated verifier independence.
   - Done: all three campaign falsifiers fail closed.
3. **Single cryptographic Rust engine**
   - Output: one `receipts` Rust binary, BLAKE3-256, Ed25519 journal signatures, executor key, deterministic privacy projection, `doctor`.
   - Done: tamper, identity, and privacy fixtures fail for the intended reason; no `.mjs` trust decisions.
4. **Identity capture and independent outcomes**
   - Output: `session capture`, `check`, `adjudicate`, and `import-eval` over an append-only signed journal.
   - Done: asserted fixture reaches a signed independent outcome without hand-edited JSON.
5. **Calibration**
   - Output: Rust Beta-Binomial baseline and pinned PyMC trainer/export bundle with fixed eligibility gates.
   - Done: insufficient/provisional/calibrated states and broken-dataset release rejection are proven.
6. **Reliability cards**
   - Output: deterministic local scoring, consent-gated publication projection, static JSON/HTML cards, CI validation.
   - Done: opted-in fixture is byte-stable; secret fixture is rejected.
7. **Reliability Index**
   - Output: versioned, fixed-mix index with explicit ineligibility below thresholds.
   - Done: only calibrated, sufficiently sampled model-agent variants receive an index.
8. **Release verification and handoff**
   - Output: adversarial, compatibility, statistical, packaged-install, readiness, and platform-matrix receipts plus exact git identity.

## Pre-mortem

The highest-risk assumption is that current green tests measure the shipped path. Check this first by pinning the exact binary used and proving an incompatible binary fails the launch handshake before trusting readiness.

## Checkpoints

### 2026-07-14 — Orientation

- **Verified:** canonical checkout is `C:\Users\johnr\Projects\agent-receipts`, branch `main`, HEAD `36b517d`, tracking `origin/main`, clean at inspection.
- **Verified:** public remote is `https://github.com/inchwormz/agent-receipts.git`.
- **Verified:** `receipts-compiler/Cargo.toml` still points at the obsolete `inchwormz/receipts-skill` repository.
- **Verified:** Node launch paths currently search arbitrary `PATH` for `receipts-core`; identity is not yet bound.
- Next: locate the four migration commits, capture the current full-suite baseline, then port each behavior red-first.

### 2026-07-14 — Stage 0 implementation checkpoint

- **Verified measurement trap:** the machine-wide `CARGO_TARGET_DIR` reused a binary from another checkout and falsely reported three tests that were absent from canonical source. The contaminated Rust/readiness receipt was discarded. All accepted Rust results below use repository-specific isolated targets and `--locked`.
- **Verified migration 1 red:** a verified `commit:` citation was rejected by the strict gate as an unsupported source kind.
- **Verified migration 1 green:** focused commit-citation test passed after admitting the engine-verified source kind.
- **Verified migration 2 red:** temporal observations of one file failed with `declared twice with divergent hash`.
- **Verified migration 2 green:** the end-to-end temporal test plus same-record and non-file strictness invariants passed after content-qualified file source versioning.
- **Verified migration 3 red:** receipts and resolution journals were accepted as stable file evidence.
- **Verified migration 3 green:** new self-mutating citations demote visibly and all four legacy journal paths scrub cleanly.
- **Verified migration 4 red:** a packet over 1 MiB failed `absorb` with `ENOBUFS`.
- **Verified migration 4 green:** the same packet passed when internal compiler stdout was not buffered.
- **Verified intermediate suite:** 54 isolated Rust tests and 65 Node tests passed after the four migrations.
- **Verified wrong-binary reds:** the CLI executed an incompatible first-on-`PATH` binary and ignored an explicit incompatible override.
- **Verified wrong-binary greens:** ambient `PATH` is no longer consulted; explicit binaries require a matching manifest and identity handshake.
- **Verified identity:** protocol `1`, Rust engine `0.1.1`, Windows `x86_64`, dependency-lock SHA-256 `0bfc909eed2e864b0ebaa06da9254635d2f9a403c15bc8743166e59e612088eb`, pre-commit engine SHA-256 `2b78c919ee2300e6daf0770fe19e94ebaf3416223462c8d3e4dd7b0ffa2f7a06`, build commit `36b517d428bdda1b28cbc6c6a7a542e1c4aedd13`.
- **Verified packaged product:** a real npm tarball installed from disk into `C:\Users\johnr\AppData\Local\Temp\agent-receipts-packaged-20260714093738\install`; packaged `receipts 0.1.0` and `receipts ready` passed without an ambient engine.
- **Verified final pre-commit suite:** `npm test` passed 71 Node tests, the isolated Rust/readiness suite, and readiness after the last source change.
- **Campaign trust remains 0/3:** Stage 0 fixed provenance/launcher mechanisms; the three typed-trust false-greens intentionally remain the Stage 1 measure.
- Next: commit Stage 0 as one logical unit, re-run identity/readiness from the clean commit, then start the three Stage 1 falsifiers.
