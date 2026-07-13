use crate::schema::SourceRef;

pub fn lookup_sources<'a>(sources: &'a [SourceRef], query: &str) -> Vec<&'a SourceRef> {
    let lowered = query.to_ascii_lowercase();
    sources
        .iter()
        .filter(|source| {
            source.path.to_ascii_lowercase().contains(&lowered)
                || source.kind.to_ascii_lowercase().contains(&lowered)
                || source
                    .span
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .contains(&lowered)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::lookup_sources;
    use crate::schema::SourceRef;

    #[test]
    fn finds_matching_sources() {
        let sources = vec![
            SourceRef {
                source_id: "src-1".to_string(),
                path: "logs/run-1.txt".to_string(),
                kind: "log".to_string(),
                hash: "abc".to_string(),
                hash_alg: "fnv1a-64".to_string(),
                hash_basis: None,
                span: Some("1-4".to_string()),
                observed_at: "2026-04-21T00:00:00Z".to_string(),
            },
            SourceRef {
                source_id: "src-2".to_string(),
                path: "artifacts/screenshot.png".to_string(),
                kind: "image".to_string(),
                hash: "def".to_string(),
                hash_alg: "fnv1a-64".to_string(),
                hash_basis: None,
                span: None,
                observed_at: "2026-04-21T00:00:00Z".to_string(),
            },
        ];

        let matches = lookup_sources(&sources, "screenshot");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].source_id, "src-2");
    }
}
