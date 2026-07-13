pub mod compiler;
pub mod schema;

pub use compiler::artifacts::ArtifactRef;
pub use compiler::contradictions::detect_auto_contradictions;
pub use compiler::evidence::{PacketValidationError, validate_packet_sources};
pub use compiler::journal::{JournalEvent, append_decision_log, append_event_jsonl};
pub use compiler::lookup::lookup_sources;
pub use compiler::packets::{CompilerInputBundle, build_next_pass_packet};
pub use compiler::promotion::{PromotionCandidate, promotion_record_from_candidate};
pub use compiler::receipts::{
    append_receipt, git_tree_state, load_verified_receipts, store_artifact,
};
pub use compiler::signals::detect_recurring_failure_patterns;
pub use compiler::snapshot::{SnapshotValidationError, build_snapshot};
pub use compiler::trust::{PacketizableItem, TrustAssessment, TrustDecision, assess_packet_item};

#[cfg(test)]
mod contract_tests {
    use serde_json::Value;

    #[test]
    fn bundled_schemas_are_valid_json() {
        for schema in [
            include_str!("../schemas/next_pass_packet.schema.json"),
            include_str!("../schemas/snapshot.schema.json"),
            include_str!("../schemas/decision_log_record.schema.json"),
            include_str!("../schemas/promotion_record.schema.json"),
        ] {
            let parsed: Value = serde_json::from_str(schema).expect("schema must parse");
            assert_eq!(
                parsed["$schema"],
                "https://json-schema.org/draft/2020-12/schema"
            );
        }
    }

    #[test]
    fn next_pass_packet_schema_exposes_evidence_contract() {
        let parsed: Value =
            serde_json::from_str(include_str!("../schemas/next_pass_packet.schema.json"))
                .expect("schema must parse");

        let required = parsed["required"].as_array().expect("required array");
        assert!(required.iter().any(|value| value == "evidence"));
        assert!(
            required.iter().any(|value| value == "schema_version"),
            "packet schema must require schema_version"
        );
        let versions = parsed["properties"]["schema_version"]["enum"]
            .as_array()
            .expect("schema_version enum");
        assert!(
            versions.iter().any(|value| value == "2.0.0")
                && versions.iter().any(|value| value == "1.2.0")
                && versions.iter().any(|value| value == "1.1.0"),
            "packet schema_version enum must cover current + legacy versions"
        );
        assert!(parsed["$defs"]["evidenceRecord"].is_object());
        assert!(parsed["$defs"]["evidenceRecord"]["properties"]["source_refs"].is_object());
        assert!(parsed["$defs"]["verifierFinding"]["properties"]["source_refs"].is_object());
        let source_ref_required = parsed["$defs"]["sourceRef"]["required"]
            .as_array()
            .expect("sourceRef.required");
        assert!(
            source_ref_required.iter().any(|value| value == "hash_alg"),
            "sourceRef must require hash_alg"
        );
        assert_eq!(
            parsed["$defs"]["sourceRef"]["properties"]["hash"]["pattern"],
            "^(?:[0-9a-f]{16}|[0-9a-f]{64})$",
            "sourceRef.hash must cover legacy FNV and BLAKE3-256 digest shapes"
        );
        let hash_algs = parsed["$defs"]["sourceRef"]["properties"]["hash_alg"]["enum"]
            .as_array()
            .expect("hash_alg enum");
        assert!(hash_algs.iter().any(|value| value == "fnv1a-64"));
        assert!(hash_algs.iter().any(|value| value == "blake3-256"));
        assert!(
            parsed["$defs"]["haltSignal"]["required"]
                .as_array()
                .expect("halt signal required")
                .iter()
                .any(|value| value == "source_ids")
        );
    }

    #[test]
    fn snapshot_schema_exposes_source_integrity_contract() {
        let parsed: Value = serde_json::from_str(include_str!("../schemas/snapshot.schema.json"))
            .expect("snapshot schema must parse");

        let required = parsed["required"].as_array().expect("required array");
        assert!(
            required.iter().any(|value| value == "schema_version"),
            "snapshot schema must require schema_version"
        );
        assert_eq!(
            parsed["$defs"]["sourceRef"]["properties"]["hash_alg"]["enum"][0]
                .as_str()
                .unwrap_or_default(),
            "fnv1a-64",
            "snapshot sourceRef.hash_alg must enumerate fnv1a-64"
        );
    }

    #[test]
    fn decision_log_schema_matches_raw_record_contract() {
        let parsed: Value =
            serde_json::from_str(include_str!("../schemas/decision_log_record.schema.json"))
                .expect("schema must parse");

        let required = parsed["required"].as_array().expect("required array");
        assert!(required.iter().any(|value| value == "decision_kind"));
        assert!(required.iter().all(|value| value != "payload"));
    }
}
