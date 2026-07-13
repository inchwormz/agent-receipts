use crate::schema::PromotionRecord;

#[derive(Debug, Clone, PartialEq)]
pub struct PromotionCandidate {
    pub id: String,
    pub kind: String,
    pub source_ids: Vec<String>,
    pub repeated_utility_count: u32,
    pub verifier_support: bool,
    pub conflict_count: u32,
}

pub fn promotion_record_from_candidate(candidate: &PromotionCandidate) -> PromotionRecord {
    let should_promote = candidate.repeated_utility_count >= 2
        && candidate.verifier_support
        && candidate.conflict_count == 0;
    let decision = if should_promote { "allow" } else { "review" };
    let reason = if should_promote {
        format!(
            "Repeated utility observed {} times with verifier support and no conflicts.",
            candidate.repeated_utility_count
        )
    } else if candidate.conflict_count > 0 {
        format!(
            "Conflicting evidence still exists ({} conflicts); keep this candidate under review.",
            candidate.conflict_count
        )
    } else if !candidate.verifier_support {
        format!(
            "Verifier support is still missing after {} repeated observations; keep this candidate under review.",
            candidate.repeated_utility_count
        )
    } else {
        format!(
            "Needs more repeated utility before promotion (observed {} times).",
            candidate.repeated_utility_count
        )
    };

    PromotionRecord {
        id: candidate.id.clone(),
        kind: candidate.kind.clone(),
        source_ids: candidate.source_ids.clone(),
        decision: decision.to_string(),
        reason,
        expires_after_pass: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{PromotionCandidate, promotion_record_from_candidate};

    #[test]
    fn promotes_only_when_threshold_and_verifier_are_satisfied() {
        let candidate = PromotionCandidate {
            id: "promo-1".to_string(),
            kind: "promoted_directive".to_string(),
            source_ids: vec!["src-1".to_string()],
            repeated_utility_count: 3,
            verifier_support: true,
            conflict_count: 0,
        };

        let record = promotion_record_from_candidate(&candidate);
        assert_eq!(record.decision, "allow");
    }

    #[test]
    fn keeps_conflicted_candidates_under_review() {
        let candidate = PromotionCandidate {
            id: "promo-2".to_string(),
            kind: "promoted_directive".to_string(),
            source_ids: vec!["src-1".to_string()],
            repeated_utility_count: 5,
            verifier_support: true,
            conflict_count: 1,
        };

        let record = promotion_record_from_candidate(&candidate);
        assert_eq!(record.decision, "review");
    }

    #[test]
    fn reports_missing_verifier_support_explicitly() {
        let candidate = PromotionCandidate {
            id: "promo-3".to_string(),
            kind: "promoted_directive".to_string(),
            source_ids: vec!["src-1".to_string()],
            repeated_utility_count: 4,
            verifier_support: false,
            conflict_count: 0,
        };

        let record = promotion_record_from_candidate(&candidate);
        assert!(record.reason.contains("Verifier support is still missing"));
    }
}
