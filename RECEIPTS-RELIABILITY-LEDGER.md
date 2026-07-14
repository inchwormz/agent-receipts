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

### 2026-07-14 — Stage 0 committed identity

- **Verified commit:** `052329d14edb52806440f0e770c1de560e28dac7` (`feat: make public receipts checkout self-verifying`). Not pushed.
- **Verified committed engine SHA-256:** `3d94c3b296865609d36d3754ca20699b95bd1e336826b71c98fede6b07ecb9df`.
- **Verified committed identity:** protocol `1`, engine `0.1.1`, build commit `052329d14edb52806440f0e770c1de560e28dac7`, dependency-lock SHA-256 `0bfc909eed2e864b0ebaa06da9254635d2f9a403c15bc8743166e59e612088eb`, Windows `x86_64`.

### 2026-07-14 — Stage 1 typed-trust checkpoint

- **Trust moved from 0/3 to 3/3 demonstrated false-green mechanisms blocked.** A failed command cannot improve Evidence Coverage; a covered subject change makes an old green `stale`; self-authored and zero-score verifier input cannot promote a claim.
- **Verified schema:** current output is `2.0.0`; legacy `1.1.0` and `1.2.0` deserialize without being rewritten. Agent `confidence` is read only as diagnostic `reported_confidence`.
- **Verified claim/event separation:** execution and work receipts render as typed `receipt_events`, never claim coverage or trusted facts. The report heading is **Evidence Coverage**.
- **Verified check binding:** `.receipts/checks.toml` controls tokenized command, covered path globs, eligible claim kinds, environment class, check version, and target claims. Passing claims bind to BLAKE3 digests of exact covered bytes, root dependency locks, command, and OS/architecture environment.
- **Verified freshness:** changing a covered file removes promotion and emits `applicability: stale`; a check attempt written after compile makes the packet fingerprint stale.
- **Verified negative controls:** an intended broken fixture records `expected_failure` only when it exits nonzero with the declared signature. An unexpected pass or wrong signature records `failed` and exits red.
- **Verified retry visibility:** first result, latest result, total attempts, attempts-to-green, failure signatures, transitions, and flake rate remain in `check_histories`; retry-until-green no longer erases red history.
- **Verified sidecar custody:** a copied, internally hash-valid check-attempt sidecar fails compile unless its primary and negative-control receipts exist in the same verified receipt journal and match command digest, labels, and outcomes.
- **Verified verifier rule:** `verifier_score` is ignored for trust and gate provenance. Raw verifier input is non-promoting until Stage 2 can require a valid signature from a different authenticated executor principal.
- **Verified rollout mode:** stale and unbound findings appear in the HTML report as `report-only`; they are not yet independent categorical gate failures.
- **Verified final Node/readiness result:** `npm test` passed **78/78 Node tests** and `receipts readiness: passed` after the last trust-code change.
- **Verified final Rust result:** isolated `cargo test --locked` passed **57 tests total** (49 library + 1 determinism + 1 init + 4 argv + 2 compatibility); formatter check and `git diff --check` passed.
- **Calibration remains 0 independently adjudicated outcomes.** No false-green probability or reliability score is eligible for presentation.
- Next: commit Stage 1 as one logical unit, re-verify the committed engine identity, then begin Stage 2 cryptographic signing and the single-engine migration.

### 2026-07-14 — Stage 2 cryptographic single-engine checkpoint

- **Verified one authority:** new execution records are strict signed V2 envelopes in the existing `receipts/receipts.jsonl`; the rejected parallel-sidecar design was removed. Frozen V1 FNV lines remain byte-compatible and always load as `legacy_weak`; a V1 line after V2 is a fail-closed downgrade.
- **Verified signed binding:** BLAKE3-256 and Ed25519 cover run ID, record kind, sequence, typed previous digest, frozen payload and artifact digests, executor public key/fingerprint, binary digest, build commit, dependency-lock digest, protocol, OS, and architecture. Digest length alone never assigns integrity.
- **Verified journal adversaries:** payload/signature edits, an unsigned random 64-character digest, cross-run replay, and signed-tail truncation all fail for their named hash, signature, run-binding, or pinned-head reason.
- **Verified artifacts:** new stdout/stderr artifacts use BLAKE3-256 and are required, regular non-symlink files whose bytes are re-hashed before compile or report consumption. Byte tampering fails closed.
- **Verified executor identity:** a local Ed25519 key is generated outside repositories, protected by user-only permissions, and audited by a sign/verify challenge. `receipts doctor` also verifies executable/build/lock/platform identity, schema support, and `.receipts/checks.toml`; unresolved model metadata stays explicitly `unavailable` and cannot receive a model score.
- **Verified privacy boundary:** `receipts project-public` recompiles current disk state and constructs a deterministic aggregate from a fixed allowlist. The fixture was byte-stable and omitted prompts, stdout, tokens, repository URLs, source IDs, commands, and absolute paths.
- **Verified single Rust product path:** the binary is now named `receipts`. Rust owns ingest, run/check/diff, compile, synthesis, gate, resolve, reports, briefs, absorb/conclude composites, doctor, public projection, and readiness. The shipped Node file is a small npm dispatcher/identity launcher; legacy `.mjs` implementations are not packaged or reachable from product commands.
- **Verified direct-build identity:** repository Cargo builds derive exact Git HEAD and Cargo.lock SHA-256 when explicit build variables are absent; unresolved identity still refuses to sign.
- **Verified full compatibility:** `npm run test:node` passed **86/86 Node tests** after the last engine migration change.
- **Verified full Rust:** isolated `cargo test --locked` passed **58 tests total** (50 library + 1 determinism + 1 init + 4 argv + 2 legacy/schema compatibility).
- **Verified packaged checkout:** npm tarball contained 54 files and no trust-bearing scripts; install, `doctor`, and Rust `ready` passed from `C:\Users\johnr\AppData\Local\Temp\agent-receipts-stage2-package-20260714105931` with binary BLAKE3 `4b3dc80b24eb028d654f36fdffa3be1cabf8b2dfc84cc8eefbe798811ed4f7f4`.
- **Threat boundary documented:** signatures establish executor continuity and post-hoc tamper evidence; they do not prove semantic truth, create verifier independence, or defend a host compromised while the engine runs.
- Next: commit Stage 2 as one logical unit, verify the committed binary identity/readiness, then add exact model/session capture and independently adjudicated signed outcomes.

### 2026-07-14 — Stage 3 identity and outcome checkpoint

- **Verified red/green:** the exact-session fixture first failed because `session` was not a product command; it now captures a resolved generic snapshot and an unresolved mutable alias without guessing. Model-specific eligibility requires explicit provider, resolved snapshot, agent name, and agent version; unresolved aliases and incomplete agent identities are explicitly ineligible.
- **Verified signed journals:** session captures and independent outcomes use strict append-only BLAKE3/Ed25519 records with typed previous digests, exact run/executor/engine bindings, and signed terminal heads. Changing a recorded success to failure makes the next append fail with `signed independent_outcome record hash mismatch at entry 1`.
- **Verified end-to-end outcome:** an asserted claim moved through an engine-declared subject-bound check into a signed `signed-human-review` outcome without hand-editing JSON. The outcome includes task/claim IDs, cited-source digest, exact model/agent/scaffold identity, adjudicator identity, task family, change size, check strength, environment match, and full retry history.
- **Verified eligibility boundary:** only success/failure results backed by independent hidden tests, benchmark adjudication, signed human review, or equivalent independent evidence are training-eligible, and only with an exact model-agent identity. An `unknown` result is always excluded. Merge/revert/incident-free signals are supporting only; self-report, bare gates/labels, and model-card claims are excluded.
- **Verified import red/green:** `import-eval` first failed as an unknown command. It now validates a strict pinned file, preserves its exact bytes under a BLAKE3 dataset hash, signs a provenance descriptor, rejects duplicate task IDs/imports, assigns task-level external results weight `0.25`, and assigns model-card metadata weight `0`.
- **Verified focused adversarial suite:** exact capture/outcome, unresolved alias, changed signed outcome, pinned import, and duplicate import all passed their expected green or fail-closed assertions (**3/3 focused Node tests** plus the Rust receipt-tamper unit test).
- **Verified full Node/readiness result:** `npm test` passed **89/89 Node tests** and `receipts readiness: passed` after the final Stage 3 source and documentation changes.
- **Verified isolated Rust result:** `cargo test --locked` with `CARGO_TARGET_DIR=C:\Users\johnr\AppData\Local\Temp\agent-receipts-stage3-final-rust-20260714` passed **59 tests total** (51 library + 1 determinism + 1 init + 4 argv + 2 compatibility) after the final eligibility fix.
- **Verified packaged checkout:** npm tarball contained 58 files; install, `doctor`, and `ready` passed from `C:\Users\johnr\AppData\Local\Temp\agent-receipts-stage3-package-20260714111702`. Pre-commit packaged binary SHA-256 was `596065b40de5f34c58dfa0f361989e2b467a368e517dad506cd8b5d958c043a0`.
- **Calibration production count remains 0.** One temporary independently adjudicated fixture proves the mechanism, but it is not retained as campaign data and no probability is eligible for presentation.
- Next: commit Stage 3 as one logical unit, verify the committed engine identity/readiness, then implement the local Beta-Binomial baseline and pinned offline trainer.

### 2026-07-14 — Stage 4 calibration checkpoint

- **Verified no-data red/green:** `calibration` first failed as an unknown command. A signed zero-outcome bundle now reports `insufficient_data` and serializes both headline probability and upper 95% false-green risk as `null`; changing its state breaks the signed bundle hash.
- **Verified baseline:** exact provider/model-snapshot/agent-version/task-family cohorts use a `Beta(1,1)` prior. Fewer than 30 effective outcomes suppress probability; exactly 30 exposes a `provisional` posterior and upper 95% bound, never `calibrated` without held-out metrics.
- **Verified external-data rules:** signed import receipts and original source bytes are re-verified at calibration time. Local independent outcomes weigh `1.0`, pinned task results weigh `0.25`, model cards add zero outcomes, and repeated task IDs cluster instead of inflating effective sample size.
- **Verified signed dataset:** Rust exports the complete feature rows and fixes a deterministic BLAKE3-grouped repository/task holdout before Python runs. Exact repository, language, freshness, environment, check strength, negative control, verifier independence, attempts, flakiness, and change-size features remain bound to the dataset hash.
- **Verified trainer root cause and correction:** the first real PyMC smoke fit failed because unconstrained ArviZ `1.2.0` removed the API required by PyMC `5.27.1`. Pinning ArviZ `0.22.0` fixed that exact import failure. `uv.lock`, `.python-version` (`3.12.10`), PyMC, NumPy, fixed seed `20260714`, and forced single-thread execution now define the offline toolchain.
- **Verified deterministic trainer:** two independent 10-draw smoke fits over the same signed dataset produced identical SHA-256 `18bdec3385e752ee8a2426912ad082249a3b7514274fee8018874974d7eae820`. This is a determinism smoke check, not a calibrated model release.
- **Verified Rust release authority:** Rust recomputes held-out Brier improvement, expected calibration error, calibration slope, posterior interval width, sample thresholds, split coverage, targets, weights, and the predictions generated by exported model parameters. A Python-authored metric or label cannot promote itself.
- **Verified fixed model gates:** hierarchical promotion requires 500 effective outcomes, five exact model-agent variants, and three task families with 50 outcomes each. Uninformative predictions fail; a controlled calibrated shape passes the 5% Brier improvement, ECE `<=0.05`, and slope `0.8–1.2` checks.
- **Verified full Node/readiness result:** the final Stage 4 tree passed **92/92 Node tests** and `receipts readiness: passed`.
- **Verified isolated Rust result:** `cargo test --locked` with `CARGO_TARGET_DIR=C:\Users\johnr\AppData\Local\Temp\agent-receipts-stage4-final-rust-20260714` passed **64 tests total** (56 library + 1 determinism + 1 init + 4 argv + 2 compatibility). Formatter and diff checks passed after the last source change.
- **Calibration production count remains 0.** The smoke and adversarial fixtures are temporary and ineligible; no campaign probability is presented.
- Next: commit Stage 4, verify the committed packaged toolchain, then build run scoring, consent-gated publication, and byte-stable static reliability cards.

### 2026-07-14 — Stages 5–6 cards and index checkpoint

- **Verified categorical precedence:** the red fixture first failed because `score` was absent. It now recomputes current disk state and suppresses both false-green probability and upper bound when a critical claim is unbound; statistical output cannot turn a categorical red green.
- **Verified local score shape:** cards separate false-green posterior/upper bound from verified completion, Evidence Coverage, critical binding, first-pass success, attempts-to-green, flake rate, escalation, cost/time availability, raw/effective samples, and exact model, agent, engine, check, dataset, bundle, and methodology identities.
- **Verified consent and privacy:** `publish` requires an explicit run-bound CC BY 4.0 consent file, recomputes the score using the consent-bound calibration bundle, constructs a fixed allowlist projection, signs it with Ed25519, and rejects secret markers or private paths. Missing consent and a `github_pat_` fixture both failed closed.
- **Verified deterministic cards:** two independent publishes and two static card builds over the opted-in fixture were byte-identical. Strict decoding, signature verification, duplicate detection, licence validation, symlink refusal, secret/private-path scanning, and clean-output enforcement run before HTML/JSON generation.
- **Verified index withholding:** the red fixture first failed because `index` was absent. A provisional, suppressed variant now emits `release_status: withheld`, `eligible: false`, and no index number. Rust boundary tests prove 200 effective outcomes, three versioned families with 30 each, calibration, exact identities, and interval width `<=0.10` are mandatory.
- **Verified release pinning:** eligible releases use the fixed equal-family upper-bound formula and pin task-mix bytes, input public-card hashes, dataset hashes, calibration-bundle hashes, methodology versions, generator version, and executor signature. Missing families are not imputed and output directories must be clean so releases remain immutable.
- **Verified repository contract:** public data is repository-backed under `public-data/`, project-authored data is CC BY 4.0 while code remains MIT, and CI validates/builds on Windows, macOS, and Linux without a database, API, telemetry, upload service, or Pages deployment.
- **Verified final Node result:** after first pinning the completed source binary, `npm run test:node` passed **93/93 tests**. The preceding 90/93 run was discarded because concurrent tests observed a partially rebuilt shared identity artifact (`Unexpected end of JSON input`).
- **Verified isolated Rust result:** `cargo test --locked` with `CARGO_TARGET_DIR=C:\Users\johnr\AppData\Local\Temp\agent-receipts-stage56-commit-rust-20260714` passed **67 tests total** (59 library + 1 determinism + 1 init + 4 argv + 2 compatibility).
- **Verified readiness and package shape:** `receipts readiness: passed`; dry-run npm packing contained **66 files**, including all score/publication/index Rust modules and no test server or public consent file. Formatter and diff checks passed.
- **Not verified:** the generated cards responded HTTP 200 at `http://127.0.0.1:41737/`, but visual browser inspection did not run because the in-app browser connection failed while initializing. The localhost helper was stopped. This is a visual-proof gap, not a card correctness claim.
- **Campaign calibration remains 0 production outcomes.** Test fixtures prove gates only; no public card or Reliability Index is released.
- Next: commit this checkpoint without pushing, then re-verify committed identity/readiness. Retry browser inspection when the in-app browser surface is available.
