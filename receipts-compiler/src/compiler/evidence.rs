use crate::schema::{
    CandidateAction, CompiledFact, Contradiction, EvidenceRecord, HaltSignal, Hypothesis,
    NextPassPacket, RecurringFailurePattern, SourceRef, VerifierFinding,
};
use std::collections::HashSet;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketValidationError {
    message: String,
}

impl PacketValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for PacketValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PacketValidationError {}

pub fn validate_packet_sources(packet: &NextPassPacket) -> Result<(), PacketValidationError> {
    let registry = source_registry(&packet.sources, &packet.raw_drilldown_refs);
    validate_evidence_records(&packet.evidence, &registry)?;
    validate_fact_records(&packet.trusted_facts, &registry)?;
    validate_hypotheses(&packet.active_hypotheses, &registry)?;
    validate_contradictions(&packet.contradictions, &registry)?;
    validate_failure_patterns(&packet.recurring_failure_patterns, &registry)?;
    validate_actions(&packet.candidate_actions, &registry)?;
    validate_verifier_findings(&packet.verifier_findings, &registry)?;
    validate_halt_signals(&packet.halt_signals, &registry)?;
    Ok(())
}

fn source_registry(sources: &[SourceRef], raw_drilldown_refs: &[SourceRef]) -> HashSet<String> {
    sources
        .iter()
        .chain(raw_drilldown_refs.iter())
        .map(|source| source.source_id.clone())
        .collect()
}

fn validate_evidence_records(
    items: &[EvidenceRecord],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_record_local_source_refs(
            "evidence",
            &item.id,
            &item.source_ids,
            &item.source_refs,
        )?;
        validate_source_ids("evidence", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_fact_records(
    items: &[CompiledFact],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_source_ids("compiled_fact", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_hypotheses(
    items: &[Hypothesis],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_source_ids("hypothesis", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_contradictions(
    items: &[Contradiction],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_source_ids("contradiction", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_failure_patterns(
    items: &[RecurringFailurePattern],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_source_ids(
            "recurring_failure_pattern",
            &item.id,
            &item.source_ids,
            registry,
        )?;
    }
    Ok(())
}

fn validate_actions(
    items: &[CandidateAction],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_source_ids("candidate_action", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_verifier_findings(
    items: &[VerifierFinding],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_record_local_source_refs(
            "verifier_finding",
            &item.id,
            &item.source_ids,
            &item.source_refs,
        )?;
        validate_source_ids("verifier_finding", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_halt_signals(
    items: &[HaltSignal],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    for item in items {
        validate_source_ids("halt_signal", &item.id, &item.source_ids, registry)?;
    }
    Ok(())
}

fn validate_source_ids(
    kind: &str,
    item_id: &str,
    source_ids: &[String],
    registry: &HashSet<String>,
) -> Result<(), PacketValidationError> {
    if source_ids.is_empty() {
        return Err(PacketValidationError::new(format!(
            "{kind} `{item_id}` must reference at least one source id"
        )));
    }

    for source_id in source_ids {
        if !registry.contains(source_id) {
            return Err(PacketValidationError::new(format!(
                "{kind} `{item_id}` references unknown source id `{source_id}`"
            )));
        }
    }

    Ok(())
}

fn validate_record_local_source_refs(
    kind: &str,
    item_id: &str,
    source_ids: &[String],
    source_refs: &[SourceRef],
) -> Result<(), PacketValidationError> {
    let local_refs: HashSet<&str> = source_refs
        .iter()
        .map(|source| source.source_id.as_str())
        .collect();
    let source_ids: HashSet<&str> = source_ids.iter().map(|source| source.as_str()).collect();

    for source_ref in source_refs {
        if !source_ids.contains(source_ref.source_id.as_str()) {
            return Err(PacketValidationError::new(format!(
                "{kind} `{item_id}` declares source_ref `{}` but does not list it in source_ids",
                source_ref.source_id
            )));
        }
    }

    for source_id in source_ids {
        if is_direct_source_id(source_id) && !local_refs.contains(source_id) {
            return Err(PacketValidationError::new(format!(
                "{kind} `{item_id}` uses direct source id `{source_id}` without a matching local source_ref"
            )));
        }
    }

    Ok(())
}

pub fn is_direct_source_id(source_id: &str) -> bool {
    source_id.starts_with("file:")
        || source_id.starts_with("command:")
        || source_id.starts_with("test:")
        || source_id.starts_with("log:")
}

#[cfg(test)]
mod tests {
    use super::validate_packet_sources;
    use crate::schema::{
        CandidateAction, CompiledFact, Contradiction, EvidenceRecord, HaltSignal, Hypothesis,
        NextPassPacket, RecurringFailurePattern, SourceRef, VerifierFinding,
    };

    fn source() -> SourceRef {
        SourceRef {
            source_id: "src-1".to_string(),
            path: "evidence/log.txt".to_string(),
            kind: "log".to_string(),
            hash: "abc".to_string(),
            hash_alg: "fnv1a-64".to_string(),
            hash_basis: None,
            span: Some("1-4".to_string()),
            observed_at: "2026-04-21T00:00:00Z".to_string(),
        }
    }

    fn direct_source() -> SourceRef {
        SourceRef {
            source_id: "file:src/main.rs:10".to_string(),
            path: "src/main.rs".to_string(),
            kind: "file".to_string(),
            hash: "abc".to_string(),
            hash_alg: "fnv1a-64".to_string(),
            hash_basis: None,
            span: Some("10".to_string()),
            observed_at: "2026-04-21T00:00:00Z".to_string(),
        }
    }

    fn packet(source_ids: Vec<String>) -> NextPassPacket {
        NextPassPacket {
            schema_version: "1.1.0".to_string(),
            objective_id: "obj-1".to_string(),
            run_id: "run-1".to_string(),
            branch_id: "main".to_string(),
            pass_id: "pass-1".to_string(),
            objective: "Solve the task".to_string(),
            evidence: vec![EvidenceRecord {
                id: "ev-1".to_string(),
                kind: "observation".to_string(),
                summary: "build output".to_string(),
                source_ids: source_ids.clone(),
                source_refs: vec![],
                observed_at: "2026-04-21T00:00:00Z".to_string(),
                agent_id: None,
                lane: None,
                reported_confidence: None,
                rationale: None,
                diff_ref: None,
                span_before: None,
                span_after: None,
                claimed_agent_id: None,
                claimed_lane: None,
                provenance_warnings: vec![],
            }],
            trusted_facts: vec![CompiledFact {
                id: "fact-1".to_string(),
                statement: "build failed".to_string(),
                reported_confidence: Some(0.8),
                objective_relevance: 0.9,
                novelty_gain: 0.1,
                needs_raw_drilldown: false,
                source_ids: source_ids.clone(),
                attestation: None,
            }],
            active_hypotheses: vec![Hypothesis {
                id: "hyp-1".to_string(),
                statement: "schema is wrong".to_string(),
                reported_confidence: Some(0.6),
                verifier_score: None,
                source_ids: source_ids.clone(),
            }],
            contradictions: vec![Contradiction {
                id: "con-1".to_string(),
                summary: "two packet versions disagree".to_string(),
                conflicting_item_ids: vec!["fact-1".to_string()],
                severity: "medium".to_string(),
                source_ids: source_ids.clone(),
                source_refs: None,
            }],
            recurring_failure_patterns: vec![RecurringFailurePattern {
                id: "pat-1".to_string(),
                summary: "build keeps failing".to_string(),
                count: 2,
                last_seen_at: "2026-04-21T00:00:00Z".to_string(),
                impact: "medium".to_string(),
                source_ids: source_ids.clone(),
            }],
            candidate_actions: vec![CandidateAction {
                id: "act-1".to_string(),
                title: "fix build".to_string(),
                rationale: "clear next step".to_string(),
                actionability_score: 0.9,
                decision_dependency_ids: vec![],
                source_ids: source_ids.clone(),
                category: None,
                blocking: None,
                resolved: None,
                resolution_id: None,
                suggested_argv: vec![],
            }],
            verifier_findings: vec![VerifierFinding {
                id: "ver-1".to_string(),
                summary: "build failed".to_string(),
                status: "failed".to_string(),
                verifier_score: 0.0,
                source_ids: source_ids.clone(),
                source_refs: vec![],
                agent_id: None,
                lane: None,
                closure_reason: None,
                finding_kind: None,
                claimed_agent_id: None,
                claimed_lane: None,
                provenance_warnings: vec![],
            }],
            open_questions: vec![],
            raw_drilldown_refs: vec![source()],
            halt_signals: vec![HaltSignal {
                id: "halt-1".to_string(),
                kind: "continue".to_string(),
                contribution: 0.1,
                rationale: "still unresolved".to_string(),
                source_ids,
            }],
            sources: vec![source()],
            lane_digests: vec![],
            trust_assessments: vec![],
            receipt_events: vec![],
            evidence_coverage: None,
            check_histories: vec![],
        }
    }

    #[test]
    fn accepts_packets_when_all_source_refs_resolve() {
        let result = validate_packet_sources(&packet(vec!["src-1".to_string()]));
        assert!(result.is_ok());
    }

    #[test]
    fn rejects_packets_when_source_ids_are_missing() {
        let result = validate_packet_sources(&packet(vec![]));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_packets_with_unknown_source_ids() {
        let result = validate_packet_sources(&packet(vec!["missing".to_string()]));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_direct_source_ids_without_local_source_refs() {
        let direct = direct_source();
        let mut packet = packet(vec![direct.source_id.clone()]);
        packet.sources.push(direct);

        let result = validate_packet_sources(&packet);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("without a matching local source_ref")
        );
    }

    #[test]
    fn accepts_direct_source_ids_with_local_source_refs() {
        let direct = direct_source();
        let mut packet = packet(vec![direct.source_id.clone()]);
        packet.evidence[0].source_refs = vec![direct.clone()];
        packet.verifier_findings[0].source_refs = vec![direct.clone()];
        packet.sources.push(direct);

        let result = validate_packet_sources(&packet);

        assert!(result.is_ok());
    }
}
