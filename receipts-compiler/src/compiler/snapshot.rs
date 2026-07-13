use crate::compiler::artifacts::{ArtifactRef, artifact_registry};
use crate::schema::{RECEIPTS_SCHEMA_VERSION, Snapshot, SnapshotInput, StateDelta, WorkerResult};
use std::collections::HashSet;
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotValidationError {
    message: String,
}

impl SnapshotValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for SnapshotValidationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SnapshotValidationError {}

pub fn build_snapshot(
    run_id: impl Into<String>,
    pass_id: impl Into<String>,
    branch_id: impl Into<String>,
    created_at: impl Into<String>,
    inputs: Vec<SnapshotInput>,
    worker_results: Vec<WorkerResult>,
    state_delta: Vec<StateDelta>,
    artifact_refs: Vec<ArtifactRef>,
) -> Result<Snapshot, SnapshotValidationError> {
    let registry = artifact_registry(&artifact_refs);
    validate_snapshot_refs(&inputs, &worker_results, &registry)?;

    Ok(Snapshot {
        schema_version: RECEIPTS_SCHEMA_VERSION.to_string(),
        run_id: run_id.into(),
        pass_id: pass_id.into(),
        branch_id: branch_id.into(),
        created_at: created_at.into(),
        inputs,
        worker_results,
        state_delta,
        artifact_refs,
    })
}

fn validate_snapshot_refs(
    inputs: &[SnapshotInput],
    worker_results: &[WorkerResult],
    registry: &HashSet<String>,
) -> Result<(), SnapshotValidationError> {
    for input in inputs {
        for ref_id in &input.ref_ids {
            if !registry.contains(ref_id) {
                return Err(SnapshotValidationError::new(format!(
                    "snapshot input `{}` references unknown artifact `{}`",
                    input.id, ref_id
                )));
            }
        }
    }

    for result in worker_results {
        for output_id in &result.output_ids {
            if !registry.contains(output_id) {
                return Err(SnapshotValidationError::new(format!(
                    "worker result `{}` references unknown artifact `{}`",
                    result.id, output_id
                )));
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_snapshot;
    use crate::compiler::artifacts::ArtifactRef;
    use crate::schema::{SnapshotInput, StateDelta, WorkerResult};

    #[test]
    fn builds_snapshot() {
        let snapshot = build_snapshot(
            "run-1",
            "pass-1",
            "main",
            "2026-04-21T00:00:00Z",
            vec![SnapshotInput {
                id: "input-1".to_string(),
                kind: "task".to_string(),
                summary: "task.md".to_string(),
                ref_ids: vec!["src-1".to_string()],
            }],
            vec![WorkerResult {
                id: "worker-1".to_string(),
                worker: "local-model".to_string(),
                status: "ok".to_string(),
                output_ids: vec!["src-1".to_string()],
                notes: "produced observation".to_string(),
            }],
            vec![StateDelta {
                id: "delta-1".to_string(),
                kind: "fact-added".to_string(),
                target_id: "fact-1".to_string(),
                summary: "added fact".to_string(),
            }],
            vec![ArtifactRef {
                source_id: "src-1".to_string(),
                path: "artifacts/task.md".to_string(),
                kind: "document".to_string(),
                hash: "abc".to_string(),
                hash_alg: "fnv1a-64".to_string(),
                hash_basis: None,
                span: None,
                observed_at: "2026-04-21T00:00:00Z".to_string(),
            }],
        )
        .expect("valid snapshot");

        assert_eq!(snapshot.run_id, "run-1");
        assert_eq!(snapshot.branch_id, "main");
    }

    #[test]
    fn rejects_unknown_snapshot_refs() {
        let result = build_snapshot(
            "run-1",
            "pass-1",
            "main",
            "2026-04-21T00:00:00Z",
            vec![SnapshotInput {
                id: "input-1".to_string(),
                kind: "task".to_string(),
                summary: "task.md".to_string(),
                ref_ids: vec!["missing".to_string()],
            }],
            vec![],
            vec![],
            vec![],
        );

        assert!(result.is_err());
    }
}
