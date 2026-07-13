#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustDecision {
    Allow,
    Deny,
    Review,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketizableItem {
    pub kind: String,
    pub contains_secret: bool,
    pub contains_pii: bool,
    pub needs_raw_drilldown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustAssessment {
    pub decision: TrustDecision,
    pub reason: String,
}

pub fn assess_packet_item(item: &PacketizableItem) -> TrustAssessment {
    if item.contains_secret || item.contains_pii {
        return TrustAssessment {
            decision: TrustDecision::Deny,
            reason: "Item contains secret or PII material and must not be packetized.".to_string(),
        };
    }

    if item.needs_raw_drilldown {
        return TrustAssessment {
            decision: TrustDecision::Review,
            reason: "Item is useful but requires raw drilldown before promotion into the next-pass packet."
                .to_string(),
        };
    }

    TrustAssessment {
        decision: TrustDecision::Allow,
        reason: format!("{} is safe to packetize.", item.kind),
    }
}

#[cfg(test)]
mod tests {
    use super::{PacketizableItem, TrustDecision, assess_packet_item};

    #[test]
    fn denies_secret_material() {
        let assessment = assess_packet_item(&PacketizableItem {
            kind: "compiled_fact".to_string(),
            contains_secret: true,
            contains_pii: false,
            needs_raw_drilldown: false,
        });

        assert_eq!(assessment.decision, TrustDecision::Deny);
    }

    #[test]
    fn reviews_items_that_need_drilldown() {
        let assessment = assess_packet_item(&PacketizableItem {
            kind: "hypothesis".to_string(),
            contains_secret: false,
            contains_pii: false,
            needs_raw_drilldown: true,
        });

        assert_eq!(assessment.decision, TrustDecision::Review);
    }
}
