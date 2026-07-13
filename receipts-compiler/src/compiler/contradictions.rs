//! Same-span divergent-summary contradiction detection.
//!
//! Scans evidence records for pairs that share at least one direct-form
//! `source_ids` entry (prefix `file:`, `command:`, `test:`) but disagree on
//! summary text AND on either `agent_id`, `lane`, or `kind`. Emits a
//! `Contradiction` entry per disagreeing pair so Prime sees the conflict in
//! the recompiled packet rather than having to replay worker-results history.
//!
//! Heuristic is deliberately conservative:
//! - Exact-duplicate summaries never fire (those are dedupe candidates, not
//!   contradictions).
//! - Pairs fire when the two summaries differ AND either they have different
//!   `kind` values OR the normalized token overlap is below 50%.
//! - Pairs must come from different agent_id/lane attribution (so the same
//!   worker repeating itself is not self-contradicting).
//!
//! Exposed as a standalone function so strict-gate/future tests can reuse the
//! same heuristic without re-running the full compiler.
use crate::schema::{Contradiction, EvidenceRecord, SourceRef};
use std::collections::BTreeSet;

const DIRECT_PREFIXES: &[&str] = &["file:", "command:", "test:"];
const TOKEN_OVERLAP_FLOOR: f64 = 0.5;

// G4: kind-pairs that are NOT contradictions — they describe upstream/downstream
// or infrastructure relationships rather than disagreement. An ordered pair
// `(a, b)` is non-contradicting if either `(a, b)` or `(b, a)` matches.
fn is_non_contradicting_pair(a: &str, b: &str) -> bool {
    let (left, right) = if a <= b { (a, b) } else { (b, a) };
    // Normalized pair (alphabetically first, second) — always check in this
    // order so the pair set is compact.
    matches!(
        (left, right),
        ("observation", "proposal")
            | ("observation", "root-cause")
            | ("gap", "proposal")
            | ("missing-check", "proposal")
            | ("root-cause", "symptom")
            | ("failure-mode", "proposal")
            | ("proposal", "risk")
    )
}

// G4: kinds that never contradict anything — they're infrastructure or seed
// records, not substantive disagreements.
fn is_infrastructure_kind(kind: &str) -> bool {
    matches!(kind, "subagent-session" | "blocker" | "objective")
}

// G7: severity tiers. Concrete-change pairs land high; purely observational
// disagreements land low; mixed cases land medium.
const HIGH_SEVERITY_KINDS: &[&str] = &["code-change", "root-cause", "test-change"];
const LOW_SEVERITY_KINDS: &[&str] = &["observation", "measurement", "symptom"];

fn severity_for_pair(left_kind: &str, right_kind: &str) -> &'static str {
    let left_high = HIGH_SEVERITY_KINDS.contains(&left_kind);
    let right_high = HIGH_SEVERITY_KINDS.contains(&right_kind);
    if left_high && right_high {
        return "high";
    }
    let left_low = LOW_SEVERITY_KINDS.contains(&left_kind);
    let right_low = LOW_SEVERITY_KINDS.contains(&right_kind);
    if left_low && right_low {
        return "low";
    }
    // Mixed case: a high-severity kind vs anything non-observation (e.g.
    // code-change vs proposal) still deserves medium. A low-severity vs
    // mixed-other case also defaults to medium.
    "medium"
}

fn is_direct_source_id(source_id: &str) -> bool {
    DIRECT_PREFIXES
        .iter()
        .any(|prefix| source_id.starts_with(prefix))
}

fn direct_source_set(record: &EvidenceRecord) -> BTreeSet<String> {
    record
        .source_ids
        .iter()
        .filter(|id| is_direct_source_id(id))
        .cloned()
        .collect()
}

fn tokens(summary: &str) -> BTreeSet<String> {
    summary
        .to_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_string())
        .collect()
}

fn token_overlap_ratio(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersect = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 { 1.0 } else { intersect / union }
}

fn different_attribution(a: &EvidenceRecord, b: &EvidenceRecord) -> bool {
    let agent_diff = match (a.agent_id.as_deref(), b.agent_id.as_deref()) {
        (Some(left), Some(right)) => left != right,
        _ => false,
    };
    let lane_diff = match (a.lane.as_deref(), b.lane.as_deref()) {
        (Some(left), Some(right)) => left != right,
        _ => false,
    };
    agent_diff || lane_diff
}

/// Build a deterministic contradiction id from the two evidence ids. Ordering
/// is stable because we always sort the pair before formatting, so the same
/// two ids produce the same id across compiles.
fn contradiction_id(left: &str, right: &str) -> String {
    let (first, second) = if left <= right {
        (left, right)
    } else {
        (right, left)
    };
    format!("con:auto:{first}:{second}")
}

/// Detect same-span divergent-summary contradictions across a slice of
/// evidence records. Returns a deterministic list sorted by contradiction id.
pub fn detect_auto_contradictions(evidence: &[EvidenceRecord]) -> Vec<Contradiction> {
    let mut out: Vec<Contradiction> = Vec::new();
    let mut emitted: BTreeSet<String> = BTreeSet::new();

    for left_idx in 0..evidence.len() {
        for right_idx in (left_idx + 1)..evidence.len() {
            let left = &evidence[left_idx];
            let right = &evidence[right_idx];

            // Skip exact-duplicate summaries; that is a dedupe candidate, not
            // a disagreement we want to surface as a contradiction.
            if left.summary == right.summary {
                continue;
            }

            // Field tuning (2026-07-13 loop field run): HARVESTED records are
            // prose sentences that MENTION a span, not assertions about its
            // content - two different sentences citing the same file lines
            // are complementary narrative, not disagreement. All six noise
            // adjudications in the field run were exactly this class. Real
            // conflicts between prose lanes still surface via receipts
            // (refutation) and Prime's read of the digests.
            if left.rationale.as_deref() == Some("harvested-from-prose")
                || right.rationale.as_deref() == Some("harvested-from-prose")
            {
                continue;
            }

            // G4: skip kind-pairs that describe upstream/downstream flow
            // rather than disagreement (observation+proposal, symptom+
            // root-cause, gap+proposal, etc.). Also skip any pair where at
            // least one side is an infrastructure kind (subagent-session,
            // blocker, objective) — those never contradict by design.
            if is_infrastructure_kind(&left.kind) || is_infrastructure_kind(&right.kind) {
                continue;
            }
            if is_non_contradicting_pair(&left.kind, &right.kind) {
                continue;
            }

            let left_sources = direct_source_set(left);
            if left_sources.is_empty() {
                continue;
            }
            let right_sources = direct_source_set(right);
            if right_sources.is_empty() {
                continue;
            }
            let shared: BTreeSet<String> =
                left_sources.intersection(&right_sources).cloned().collect();
            if shared.is_empty() {
                continue;
            }

            // Conservative: only fire when the two records come from different
            // worker attribution (agent_id or lane).
            if !different_attribution(left, right) {
                continue;
            }

            let kind_diff = left.kind != right.kind;
            let overlap = token_overlap_ratio(&tokens(&left.summary), &tokens(&right.summary));
            let summary_diverges = kind_diff || overlap < TOKEN_OVERLAP_FLOOR;
            if !summary_diverges {
                continue;
            }

            let id = contradiction_id(&left.id, &right.id);
            if !emitted.insert(id.clone()) {
                continue;
            }

            let (first_id, second_id) = if left.id <= right.id {
                (left.id.clone(), right.id.clone())
            } else {
                (right.id.clone(), left.id.clone())
            };

            let summary = format!(
                "Evidence {first_id} and {second_id} disagree on the same direct span ({}): \"{}\" vs \"{}\"",
                shared.iter().cloned().collect::<Vec<_>>().join(", "),
                left.summary,
                right.summary
            );

            let mut source_ids: Vec<String> = shared.iter().cloned().collect();
            source_ids.sort();

            // G5: build source_refs for the contradiction by taking the union
            // of the two conflicting records' source_refs restricted to the
            // shared direct-span entries. That lets strict-gate re-verify the
            // file hash the compiler computed.
            let mut collected_refs: Vec<SourceRef> = Vec::new();
            let mut seen_ref_ids: BTreeSet<String> = BTreeSet::new();
            for candidate in left.source_refs.iter().chain(right.source_refs.iter()) {
                if !shared.contains(&candidate.source_id) {
                    continue;
                }
                if !seen_ref_ids.insert(candidate.source_id.clone()) {
                    continue;
                }
                collected_refs.push(candidate.clone());
            }
            collected_refs.sort_by(|a, b| a.source_id.cmp(&b.source_id));
            let source_refs = if collected_refs.is_empty() {
                None
            } else {
                Some(collected_refs)
            };

            // G7: pick severity from the (left.kind, right.kind) pair.
            let severity = severity_for_pair(&left.kind, &right.kind).to_string();

            out.push(Contradiction {
                id,
                summary,
                conflicting_item_ids: vec![first_id, second_id],
                severity,
                source_ids,
                source_refs,
            });
        }
    }

    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::detect_auto_contradictions;
    use crate::schema::EvidenceRecord;

    fn record(
        id: &str,
        kind: &str,
        summary: &str,
        source_ids: &[&str],
        agent: Option<&str>,
        lane: Option<&str>,
    ) -> EvidenceRecord {
        EvidenceRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            summary: summary.to_string(),
            source_ids: source_ids.iter().map(|s| (*s).to_string()).collect(),
            source_refs: vec![],
            observed_at: "2026-04-21T00:00:00Z".to_string(),
            agent_id: agent.map(|s| s.to_string()),
            lane: lane.map(|s| s.to_string()),
            reported_confidence: None,
            rationale: None,
            diff_ref: None,
            span_before: None,
            span_after: None,
            claimed_agent_id: None,
            claimed_lane: None,
            provenance_warnings: vec![],
        }
    }

    #[test]
    fn fires_on_same_span_different_kind_and_agent() {
        let left = record(
            "ev-a",
            "code-change",
            "Patched strict-gate.mjs:42 to require direct refs",
            &["file:scripts/strict-gate.mjs:42"],
            Some("agent-alice"),
            Some("impl"),
        );
        let right = record(
            "ev-b",
            "observation",
            "strict-gate.mjs:42 already validates directly; no change needed",
            &["file:scripts/strict-gate.mjs:42"],
            Some("agent-bob"),
            Some("verify"),
        );
        let contradictions = detect_auto_contradictions(&[left, right]);
        assert_eq!(contradictions.len(), 1, "expected 1 contradiction");
        assert_eq!(contradictions[0].severity, "medium");
        assert_eq!(contradictions[0].conflicting_item_ids, vec!["ev-a", "ev-b"]);
    }

    #[test]
    fn skips_when_no_shared_direct_source() {
        let left = record(
            "ev-a",
            "code-change",
            "summary one",
            &["file:foo.rs:1"],
            Some("a"),
            None,
        );
        let right = record(
            "ev-b",
            "observation",
            "summary two",
            &["file:bar.rs:1"],
            Some("b"),
            None,
        );
        let contradictions = detect_auto_contradictions(&[left, right]);
        assert!(contradictions.is_empty());
    }

    #[test]
    fn skips_same_agent_same_lane() {
        let left = record(
            "ev-a",
            "code-change",
            "first take",
            &["file:foo.rs:1"],
            Some("alice"),
            Some("impl"),
        );
        let right = record(
            "ev-b",
            "observation",
            "different take entirely, unrelated words",
            &["file:foo.rs:1"],
            Some("alice"),
            Some("impl"),
        );
        let contradictions = detect_auto_contradictions(&[left, right]);
        assert!(
            contradictions.is_empty(),
            "same-agent same-lane pairs must not fire"
        );
    }

    #[test]
    fn skips_pairs_involving_harvested_prose() {
        let mut left = record(
            "ev-a",
            "observation",
            "the dispatcher lives here and handles absorb",
            &["file:bin/receipts.mjs:13"],
            Some("a"),
            Some("lane-a"),
        );
        left.rationale = Some("harvested-from-prose".to_string());
        let right = record(
            "ev-b",
            "observation",
            "totally different sentence about unrelated commands table",
            &["file:bin/receipts.mjs:13"],
            Some("b"),
            Some("lane-b"),
        );
        let contradictions = detect_auto_contradictions(&[left, right]);
        assert!(
            contradictions.is_empty(),
            "harvested prose mentions must never fire contradictions"
        );
    }

    #[test]
    fn skips_exact_duplicate_summaries() {
        let left = record(
            "ev-a",
            "code-change",
            "identical summary",
            &["file:foo.rs:1"],
            Some("a"),
            None,
        );
        let right = record(
            "ev-b",
            "observation",
            "identical summary",
            &["file:foo.rs:1"],
            Some("b"),
            None,
        );
        let contradictions = detect_auto_contradictions(&[left, right]);
        assert!(
            contradictions.is_empty(),
            "exact duplicates are dedupe candidates, not contradictions"
        );
    }

    #[test]
    fn skips_high_token_overlap_same_kind() {
        let left = record(
            "ev-a",
            "observation",
            "the file has twenty lines of code now",
            &["file:foo.rs:1"],
            Some("a"),
            None,
        );
        let right = record(
            "ev-b",
            "observation",
            "the file now has twenty lines of code",
            &["file:foo.rs:1"],
            Some("b"),
            None,
        );
        let contradictions = detect_auto_contradictions(&[left, right]);
        assert!(
            contradictions.is_empty(),
            "high-overlap same-kind summaries must not fire"
        );
    }
}
