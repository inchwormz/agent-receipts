use crate::schema::{RECEIPTS_HASH_ALG, SourceRef};
use std::collections::HashSet;

pub type ArtifactRef = SourceRef;

pub fn artifact_ref(
    source_id: impl Into<String>,
    path: impl Into<String>,
    kind: impl Into<String>,
    hash: impl Into<String>,
    observed_at: impl Into<String>,
) -> ArtifactRef {
    ArtifactRef {
        source_id: source_id.into(),
        path: path.into(),
        kind: kind.into(),
        hash: hash.into(),
        hash_alg: RECEIPTS_HASH_ALG.to_string(),
        hash_basis: None,
        span: None,
        observed_at: observed_at.into(),
    }
}

pub fn artifact_registry(artifacts: &[ArtifactRef]) -> HashSet<String> {
    artifacts
        .iter()
        .map(|artifact| artifact.source_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{artifact_ref, artifact_registry};

    #[test]
    fn builds_artifact_registry_from_refs() {
        let artifacts = vec![artifact_ref(
            "src-1",
            "artifacts/log.txt",
            "log",
            "abc",
            "2026-04-21T00:00:00Z",
        )];

        let registry = artifact_registry(&artifacts);
        assert!(registry.contains("src-1"));
    }
}
