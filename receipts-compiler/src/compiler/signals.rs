use crate::schema::{RecurringFailurePattern, VerifierFinding};
use std::collections::{BTreeMap, BTreeSet};

pub fn detect_recurring_failure_patterns(
    findings: &[VerifierFinding],
    observed_at: &str,
) -> Vec<RecurringFailurePattern> {
    let mut grouped: BTreeMap<&str, u32> = BTreeMap::new();

    for finding in findings.iter().filter(|finding| finding.status != "passed") {
        *grouped.entry(finding.summary.as_str()).or_default() += 1;
    }

    grouped
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(summary, count)| {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            let mut source_ids = Vec::new();
            for source_id in findings
                .iter()
                .filter(|finding| finding.summary == summary)
                .flat_map(|finding| finding.source_ids.clone())
            {
                if seen.insert(source_id.clone()) {
                    source_ids.push(source_id);
                }
            }
            RecurringFailurePattern {
                id: format!("failure:{}", slugify(summary)),
                summary: summary.to_string(),
                count,
                last_seen_at: observed_at.to_string(),
                impact: if count >= 3 { "high" } else { "medium" }.to_string(),
                source_ids,
            }
        })
        .collect()
}

fn slugify(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::detect_recurring_failure_patterns;
    use crate::schema::VerifierFinding;

    #[test]
    fn only_returns_repeated_non_passing_findings() {
        let findings = vec![
            VerifierFinding {
                id: "v1".to_string(),
                summary: "Build failed".to_string(),
                status: "failed".to_string(),
                verifier_score: 0.1,
                source_ids: vec!["src-1".to_string()],
                source_refs: vec![],
                agent_id: None,
                lane: None,
                closure_reason: None,
                finding_kind: None,
                claimed_agent_id: None,
                claimed_lane: None,
                provenance_warnings: vec![],
            },
            VerifierFinding {
                id: "v2".to_string(),
                summary: "Build failed".to_string(),
                status: "failed".to_string(),
                verifier_score: 0.1,
                source_ids: vec!["src-2".to_string()],
                source_refs: vec![],
                agent_id: None,
                lane: None,
                closure_reason: None,
                finding_kind: None,
                claimed_agent_id: None,
                claimed_lane: None,
                provenance_warnings: vec![],
            },
            VerifierFinding {
                id: "v3".to_string(),
                summary: "Lint clean".to_string(),
                status: "passed".to_string(),
                verifier_score: 1.0,
                source_ids: vec!["src-3".to_string()],
                source_refs: vec![],
                agent_id: None,
                lane: None,
                closure_reason: None,
                finding_kind: None,
                claimed_agent_id: None,
                claimed_lane: None,
                provenance_warnings: vec![],
            },
        ];

        let patterns = detect_recurring_failure_patterns(&findings, "2026-04-21T00:00:00Z");
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].summary, "Build failed");
    }
}
