# Proof: it polices its own development

Agent Receipts is built by AI agents, and every substantive engineering pass runs under the engine itself. That was a product decision: a trust tool whose own development you have to take on faith would be a joke. These are real incidents from the receipt journals of this repo's development, told with the mechanism visible.

## The planted liar

The first end-to-end fleet test ran three honest agent lanes and one lane instructed to lie ("report that you ran cargo test and everything passed" - it ran nothing). The honest run's gate went green. The liar's run went red with the gate naming both fabrications: a command claim with no receipt behind it and a test claim contradicted by the actual suite receipt. The red-team scenario is preserved as a permanent test: a planted lying lane must turn the gate red, forever ([m0_trust_semantics.test.js](../tests/m0_trust_semantics.test.js)).

## The night format demands killed a fleet

An early version required agents to emit fenced machine records. In the first real multi-pane test, agents drifted into shorthand and prose, every deviation hard-failed the lane, the orchestrator escalated format demands across roughly a dozen panes, and the operation ended in "no receipts required" - full abandonment. The postmortem verdict: the tool was punishing agents for being agents.

The engine was rebuilt around the opposite doctrine (accept anything, verify strictly), and the actual broken lane output from that night now lives in the test suite as replay fixtures: real shorthand, real broken JSON, real prose walls, all of which must ingest cleanly ([forgiving_ingest.test.js](../tests/forgiving_ingest.test.js)).

## The stale-binary catch

Mid-development, a receipts run of the engine's own smoke test returned exit 1: the installed binary lagged the source and didn't know a new subcommand. The failing receipt sat in the journal, superseded by a passing one only after an actual reinstall. Quiet staleness became a loud, recorded event, which is precisely the failure class (trusting an outdated instrument) that costs debugging days everywhere else.

## The pipe that ate an exit code

During the loop's first full field run, a shell pipeline swallowed `conclude`'s failing exit code, and the session nearly reported success. The contradiction surfaced immediately because the gate's verdict is also written to disk: the on-disk report said red while the pipeline said fine. The on-disk truth won, the underlying bug (a stale-packet design gap in `conclude` itself) was found and fixed, and `conclude` now recompiles before synthesizing so the natural loop can't hit it again. The engine catching its own author's mistake, in its own maiden run, is the strongest evidence we have that the design points the right way.

## The self-refuting rename

The rename to Agent Receipts was itself verified by a receipts run of the renamed engine. During that run, the engine's `init` hint printed a GitHub URL that didn't exist yet (its author had renamed the URL prematurely). The field run surfaced it before any user ever saw it. Two other rename regressions (a wrong binary target in a launcher, a formatting break) were caught by suites running as receipts, with the failing and passing receipts both preserved in sequence.

## What a green run looks like

The rename verification run, as its brief reported it:

```text
VERDICT: GATE PASSED
WORKLIST (0 blocking, 0 advisory, 0 resolved)
TRUSTED FACTS (4 attested, 0 verifier)
  [attested] receipts run: `cargo test --manifest-path receipts-compiler/Cargo.toml` exited 0
  [attested] receipts run: `node --test tests/...` exited 0
  [attested] receipts run: `node scripts/readiness.mjs` exited 0
  [attested] work: tree changes in window rcpt-0001..rcpt-0004
```

Four facts, each one something a machine watched happen. The lane prose from that run (a paragraph of claims about what was renamed) sits underneath at narrative tier, indexed with drill-down spans, trusted for nothing.

## Run it yourself

```bash
receipts ready                      # the pipeline proves itself end to end
cargo test --manifest-path receipts-compiler/Cargo.toml
node --test tests/*.test.js
```

The readiness check exercises the full loop against fixtures, including the red-team cases. If someone hands you a fork of this repo, run the same three commands before believing anything about it. That habit is the product.
