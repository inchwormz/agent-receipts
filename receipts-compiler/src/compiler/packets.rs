use crate::compiler::evidence::{PacketValidationError, validate_packet_sources};
use crate::schema::{
    CandidateAction, CheckHistory, CompiledFact, Contradiction, EvidenceCoverage, EvidenceRecord,
    HaltSignal, Hypothesis, NextPassPacket, RECEIPTS_SCHEMA_VERSION, ReceiptEvent,
    RecurringFailurePattern, SourceRef, TrustAssessment, VerifierFinding,
};

#[derive(Debug, Clone, PartialEq)]
pub struct CompilerInputBundle {
    pub objective_id: String,
    pub run_id: String,
    pub branch_id: String,
    pub pass_id: String,
    pub objective: String,
    pub evidence: Vec<EvidenceRecord>,
    pub trusted_facts: Vec<CompiledFact>,
    pub active_hypotheses: Vec<Hypothesis>,
    pub contradictions: Vec<Contradiction>,
    pub recurring_failure_patterns: Vec<RecurringFailurePattern>,
    pub candidate_actions: Vec<CandidateAction>,
    pub verifier_findings: Vec<VerifierFinding>,
    pub open_questions: Vec<String>,
    pub raw_drilldown_refs: Vec<SourceRef>,
    pub halt_signals: Vec<HaltSignal>,
    pub sources: Vec<SourceRef>,
    pub lane_digests: Vec<crate::schema::LaneDigest>,
    pub trust_assessments: Vec<TrustAssessment>,
    pub receipt_events: Vec<ReceiptEvent>,
    pub evidence_coverage: EvidenceCoverage,
    pub check_histories: Vec<CheckHistory>,
}

pub fn build_next_pass_packet(
    input: CompilerInputBundle,
) -> Result<NextPassPacket, PacketValidationError> {
    let packet = NextPassPacket {
        schema_version: RECEIPTS_SCHEMA_VERSION.to_string(),
        objective_id: input.objective_id,
        run_id: input.run_id,
        branch_id: input.branch_id,
        pass_id: input.pass_id,
        objective: input.objective,
        evidence: input.evidence,
        trusted_facts: input.trusted_facts,
        active_hypotheses: input.active_hypotheses,
        contradictions: input.contradictions,
        recurring_failure_patterns: input.recurring_failure_patterns,
        candidate_actions: input.candidate_actions,
        verifier_findings: input.verifier_findings,
        open_questions: input.open_questions,
        raw_drilldown_refs: input.raw_drilldown_refs,
        halt_signals: input.halt_signals,
        sources: input.sources,
        lane_digests: input.lane_digests,
        trust_assessments: input.trust_assessments,
        receipt_events: input.receipt_events,
        evidence_coverage: Some(input.evidence_coverage),
        check_histories: input.check_histories,
    };

    validate_packet_sources(&packet)?;
    Ok(packet)
}

#[cfg(test)]
mod tests {
    use super::{CompilerInputBundle, build_next_pass_packet};

    #[test]
    fn builds_packet_from_compiler_input() {
        let packet = build_next_pass_packet(CompilerInputBundle {
            objective_id: "obj-1".to_string(),
            run_id: "run-1".to_string(),
            branch_id: "main".to_string(),
            pass_id: "pass-1".to_string(),
            objective: "Solve the task".to_string(),
            evidence: vec![crate::schema::EvidenceRecord {
                id: "ev-1".to_string(),
                kind: "observation".to_string(),
                summary: "shell output".to_string(),
                source_ids: vec!["src-1".to_string()],
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
            trusted_facts: vec![],
            active_hypotheses: vec![],
            contradictions: vec![],
            recurring_failure_patterns: vec![],
            candidate_actions: vec![],
            verifier_findings: vec![],
            open_questions: vec!["What failed?".to_string()],
            raw_drilldown_refs: vec![crate::schema::SourceRef {
                source_id: "src-1".to_string(),
                path: "evidence/log.txt".to_string(),
                kind: "log".to_string(),
                hash: "abc".to_string(),
                hash_alg: "fnv1a-64".to_string(),
                hash_basis: None,
                span: None,
                observed_at: "2026-04-21T00:00:00Z".to_string(),
            }],
            halt_signals: vec![crate::schema::HaltSignal {
                id: "halt-1".to_string(),
                kind: "continue".to_string(),
                contribution: 0.1,
                rationale: "still unresolved".to_string(),
                source_ids: vec!["src-1".to_string()],
            }],
            sources: vec![crate::schema::SourceRef {
                source_id: "src-1".to_string(),
                path: "evidence/log.txt".to_string(),
                kind: "log".to_string(),
                hash: "abc".to_string(),
                hash_alg: "fnv1a-64".to_string(),
                hash_basis: None,
                span: None,
                observed_at: "2026-04-21T00:00:00Z".to_string(),
            }],
            lane_digests: vec![],
            trust_assessments: vec![],
            receipt_events: vec![],
            evidence_coverage: crate::schema::EvidenceCoverage::default(),
            check_histories: vec![],
        })
        .expect("valid packet");

        assert_eq!(packet.objective_id, "obj-1");
        assert_eq!(packet.open_questions, vec!["What failed?".to_string()]);
    }
}
