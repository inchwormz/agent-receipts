use serde::{Deserialize, Serialize};

/// Current Receipts packet schema version. Bump when the `NextPassPacket` or
/// `Snapshot` shape changes in a way downstream consumers must react to.
/// 2.0.0 (2026-07-14): engine-owned typed trust and diagnostic-only reported
/// confidence. Consumers accept 1.1/1.2 as legacy read-only inputs.
pub const RECEIPTS_SCHEMA_VERSION: &str = "2.0.0";

/// Versions consumers must accept when reading packets.
pub const RECEIPTS_KNOWN_SCHEMA_VERSIONS: &[&str] = &["1.1.0", "1.2.0", "2.0.0"];

/// Canonical hash algorithm label emitted for every `SourceRef.hash` value.
/// The strict gate and ingester must reject source refs whose `hash_alg` does
/// not match, so bumping this is a breaking change for evidence in flight.
pub const RECEIPTS_HASH_ALG: &str = "fnv1a-64";

fn default_hash_alg() -> String {
    RECEIPTS_HASH_ALG.to_string()
}

fn default_schema_version() -> String {
    RECEIPTS_SCHEMA_VERSION.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRef {
    pub source_id: String,
    pub path: String,
    pub kind: String,
    pub hash: String,
    #[serde(default = "default_hash_alg")]
    pub hash_alg: String,
    /// "content" when the hash was computed from on-disk bytes, "label" when it
    /// was derived from the source_id string (command/test/log refs before
    /// receipts exist). Label-hashed refs are identity keys, NOT provenance:
    /// they never satisfy direct-anchor requirements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_basis: Option<String>,
    pub span: Option<String>,
    pub observed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompiledFact {
    pub id: String,
    pub statement: String,
    #[serde(default, alias = "confidence", skip_serializing_if = "Option::is_none")]
    pub reported_confidence: Option<f32>,
    pub objective_relevance: f32,
    pub novelty_gain: f32,
    pub needs_raw_drilldown: bool,
    pub source_ids: Vec<String>,
    /// How this fact earned trusted status: "verifier" (passed verifier
    /// finding backs it) today; "attested" (runtime receipt) from M2.
    /// Absent only on packets predating the attestation ladder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceRecord {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(default, alias = "confidence", skip_serializing_if = "Option::is_none")]
    pub reported_confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_after: Option<String>,
    /// Identity the record claimed for itself when it differed from the
    /// caller's ingest stamp. The stamped agent_id/lane always win (F4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_lane: Option<String>,
    /// Non-empty when ingest had to repair the record (e.g. span clipped to
    /// the file's real line count). Demoted records are never fact-eligible.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    #[serde(default, alias = "confidence", skip_serializing_if = "Option::is_none")]
    pub reported_confidence: Option<f32>,
    pub verifier_score: Option<f32>,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Contradiction {
    pub id: String,
    pub summary: String,
    pub conflicting_item_ids: Vec<String>,
    pub severity: String,
    pub source_ids: Vec<String>,
    /// G5: Optional source_refs so tampered files cited by a contradiction are
    /// detectable. Back-compat: serializes as absent when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<Vec<SourceRef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecurringFailurePattern {
    pub id: String,
    pub summary: String,
    pub count: u32,
    pub last_seen_at: String,
    pub impact: String,
    pub source_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateAction {
    pub id: String,
    pub title: String,
    pub rationale: String,
    pub actionability_score: f32,
    pub decision_dependency_ids: Vec<String>,
    pub source_ids: Vec<String>,
    /// Worklist fields (schema 1.2.0). category: adjudicate | unblock |
    /// resolve-finding | verify-claim | re-task-or-accept. The COMPILER is
    /// the single author of `blocking`; the gate and `receipts next` only
    /// consume it. `suggested_argv` is built exclusively from
    /// engine-validated tokens (never agent free text).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggested_argv: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VerifierFinding {
    pub id: String,
    pub summary: String,
    pub status: String,
    pub verifier_score: f32,
    pub source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    /// Optional justification stamp explaining why a `status:"passed"` finding
    /// is a bounded-audit / bounded-investigation closure rather than a
    /// genuine green. When present and non-empty, the strict gate treats the
    /// finding as explicitly closed with scope, so the string-hack prefix
    /// ("AUDIT-SCOPE PASSED:" etc.) Prime used in Pass 1/2 becomes typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure_reason: Option<String>,
    /// Typed role for infrastructure findings ("synthesis", "subagent-session",
    /// "bootstrap"). The strict gate keys exemptions on this field, never on
    /// free text in ids/summaries (F6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finding_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_lane: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HaltSignal {
    pub id: String,
    pub kind: String,
    pub contribution: f32,
    pub rationale: String,
    pub source_ids: Vec<String>,
}

/// Phase 3: per-lane reading guidance for Prime. Conservative by design
/// (review finding 5): `skip-verified` is only earned by receipt-id-cited or
/// verifier-backed promotion - label-citation attestation floors at
/// read-unverified, because a lane can bulk-cite plausible passing labels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LaneDigest {
    pub lane: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub records: u32,
    pub attested: u32,
    pub verifier: u32,
    pub asserted: u32,
    pub warnings: u32,
    pub contradictions: u32,
    /// skip-verified | read-adjudicate | read-unverified | blocked
    pub read_recommendation: String,
    /// Drill-down handles: span-suffixed raw source ids for this lane.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drill_down: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustAssessment {
    pub subject_id: String,
    pub integrity: String,
    pub outcome: String,
    pub applicability: String,
    pub claim_status: String,
    pub verifier_independent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptEvent {
    pub receipt_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub integrity: String,
    pub outcome: String,
    pub exit_code: i64,
    pub attempts_for_label: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EvidenceCoverage {
    pub total_claims: u32,
    pub verified_claims: u32,
    pub verifier_backed_claims: u32,
    pub asserted_claims: u32,
    pub refuted_claims: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckHistory {
    pub check_id: String,
    pub target_claims: Vec<String>,
    pub first_result: String,
    pub latest_result: String,
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempts_to_green: Option<u32>,
    pub failure_signatures: Vec<String>,
    pub transitions: Vec<String>,
    pub flake_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NextPassPacket {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
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
    /// Phase 3 (schema 1.2.0): per-lane reading guidance. Absent on legacy
    /// packets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lane_digests: Vec<LaneDigest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trust_assessments: Vec<TrustAssessment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipt_events: Vec<ReceiptEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_coverage: Option<EvidenceCoverage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub check_histories: Vec<CheckHistory>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotInput {
    pub id: String,
    pub kind: String,
    pub summary: String,
    pub ref_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerResult {
    pub id: String,
    pub worker: String,
    pub status: String,
    pub output_ids: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateDelta {
    pub id: String,
    pub kind: String,
    pub target_id: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub run_id: String,
    pub pass_id: String,
    pub branch_id: String,
    pub created_at: String,
    pub inputs: Vec<SnapshotInput>,
    pub worker_results: Vec<WorkerResult>,
    pub state_delta: Vec<StateDelta>,
    pub artifact_refs: Vec<SourceRef>,
}

/// Frozen V1 execution-receipt payload. Legacy journal lines use the original
/// FNV chain. Signed V2 envelopes reuse this struct as their payload with a
/// blank record_hash; the envelope owns BLAKE3, typed linkage, engine identity,
/// and Ed25519 signature. Never add fields here: legacy preimages are frozen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReceiptRecord {
    pub id: String,
    /// Optional claim label this receipt attests (e.g. "test:cargo-suite").
    /// Passed verifier findings citing this label are upgraded by a passing
    /// receipt and refuted by a failing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub cmd: Vec<String>,
    pub cwd: String,
    pub exit_code: i64,
    pub duration_ms: u64,
    pub started_at: String,
    pub ended_at: String,
    pub stdout_hash: String,
    pub stderr_hash: String,
    pub stdout_tail: String,
    pub stderr_tail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub writer: String,
    pub prev_record_hash: String,
    pub record_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromotionRecord {
    pub id: String,
    pub kind: String,
    pub source_ids: Vec<String>,
    pub decision: String,
    pub reason: String,
    pub expires_after_pass: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionLogRecord {
    pub id: String,
    pub run_id: String,
    pub pass_id: String,
    pub decision_kind: String,
    pub summary: String,
    pub source_ids: Vec<String>,
    pub selected_action_ids: Vec<String>,
    pub created_at: String,
    pub promotion: Option<PromotionRecord>,
}
