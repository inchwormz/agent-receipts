use receipts_core::schema::{
    CompiledFact, EvidenceRecord, NextPassPacket, RECEIPTS_KNOWN_SCHEMA_VERSIONS,
    RECEIPTS_SCHEMA_VERSION,
};

fn empty_legacy_packet(version: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": version,
        "objective_id": "legacy-objective",
        "run_id": "legacy-run",
        "branch_id": "main",
        "pass_id": "pass-0001",
        "objective": "read without rewriting",
        "evidence": [],
        "trusted_facts": [],
        "active_hypotheses": [],
        "contradictions": [],
        "recurring_failure_patterns": [],
        "candidate_actions": [],
        "verifier_findings": [],
        "open_questions": [],
        "raw_drilldown_refs": [],
        "halt_signals": [],
        "sources": []
    })
}

#[test]
fn schema_v2_is_current_while_legacy_packets_remain_read_only() {
    assert_eq!(RECEIPTS_SCHEMA_VERSION, "2.0.0");
    assert!(RECEIPTS_KNOWN_SCHEMA_VERSIONS.contains(&"1.1.0"));
    assert!(RECEIPTS_KNOWN_SCHEMA_VERSIONS.contains(&"1.2.0"));
    assert!(RECEIPTS_KNOWN_SCHEMA_VERSIONS.contains(&"2.0.0"));

    for version in ["1.1.0", "1.2.0"] {
        let packet: NextPassPacket = serde_json::from_value(empty_legacy_packet(version)).unwrap();
        assert_eq!(packet.schema_version, version);
        let serialized = serde_json::to_value(packet).unwrap();
        assert_eq!(serialized["schema_version"], version);
    }
}

#[test]
fn legacy_agent_confidence_is_relabelled_as_reported_only() {
    let record: EvidenceRecord = serde_json::from_value(serde_json::json!({
        "id": "ev-legacy-confidence",
        "kind": "observation",
        "summary": "agent supplied this number",
        "source_ids": ["log:legacy"],
        "observed_at": "2026-07-14T00:00:00Z",
        "confidence": 0.91
    }))
    .unwrap();
    let serialized = serde_json::to_value(record).unwrap();
    assert_eq!(serialized["reported_confidence"], 0.91);
    assert!(serialized.get("confidence").is_none());

    let fact: CompiledFact = serde_json::from_value(serde_json::json!({
        "id": "fact:legacy",
        "statement": "legacy promoted claim",
        "confidence": 0.73,
        "objective_relevance": 0.8,
        "novelty_gain": 0.3,
        "needs_raw_drilldown": false,
        "source_ids": ["log:legacy"],
        "attestation": "attested"
    }))
    .unwrap();
    let serialized_fact = serde_json::to_value(fact).unwrap();
    assert!((serialized_fact["reported_confidence"].as_f64().unwrap() - 0.73).abs() < 0.000_001);
    assert!(serialized_fact.get("confidence").is_none());
}
