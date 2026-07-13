//! Phase 3: `receipts next` - the compressed Prime brief.
//!
//! The packet is the API; this is its ergonomic read. One screen answers:
//! what blocks, what's refuted, what's proven, which lanes to read (with
//! drill-down handles into quarantined raw files), what ran, and what
//! drifted. Classification is CONSUMED from the packet (the compiler is the
//! single author); the drift section renders from state/gate-report.json
//! when present rather than re-deriving live-tree state.

use crate::compiler::receipts::load_verified_receipts;
use crate::compiler::report::GateReport;
use crate::schema::NextPassPacket;
use std::fs;
use std::path::Path;

pub fn generate_brief(run_dir: &Path, as_json: bool) -> Result<String, Box<dyn std::error::Error>> {
    let packet: NextPassPacket = serde_json::from_str(&fs::read_to_string(
        run_dir.join("state").join("next_pass_packet.json"),
    )?)?;
    let receipts = load_verified_receipts(run_dir)?;
    let gate: Option<GateReport> =
        fs::read_to_string(run_dir.join("state").join("gate-report.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());

    let drift_warnings: Vec<String> = gate
        .as_ref()
        .map(|report| {
            report
                .warnings
                .iter()
                .filter(|warning| warning.contains("citation drifted"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    if as_json {
        let refutations: Vec<_> = packet
            .contradictions
            .iter()
            .filter(|c| c.id.starts_with("con:receipt:"))
            .collect();
        return Ok(serde_json::to_string_pretty(&serde_json::json!({
            "objective": packet.objective,
            "run_id": packet.run_id,
            "pass_id": packet.pass_id,
            "verdict": gate.as_ref().map(|g| if g.ok { "gate-passed" } else { "gate-failed" }).unwrap_or("gate-not-recorded"),
            "gate_errors": gate.as_ref().map(|g| g.errors.clone()).unwrap_or_default(),
            "worklist": packet.candidate_actions,
            "refutations": refutations,
            "trusted_facts": packet.trusted_facts,
            "lane_digests": packet.lane_digests,
            "receipts": receipts.iter().map(|r| serde_json::json!({
                "id": r.id, "label": r.label, "exit_code": r.exit_code,
                "duration_ms": r.duration_ms, "cmd": r.cmd.join(" "),
            })).collect::<Vec<_>>(),
            "drift_warnings": drift_warnings,
        }))?);
    }

    let mut out = String::new();
    out.push_str(&format!(
        "RECEIPTS BRIEF - {} [{} / {}]\n",
        packet.objective, packet.run_id, packet.pass_id
    ));
    out.push_str(&match gate.as_ref() {
        Some(g) if g.ok => "VERDICT: GATE PASSED\n".to_string(),
        Some(g) => format!("VERDICT: GATE FAILED ({} error(s))\n", g.errors.len()),
        None => "VERDICT: gate not recorded - run receipts gate\n".to_string(),
    });

    let blocking: Vec<_> = packet
        .candidate_actions
        .iter()
        .filter(|item| item.blocking == Some(true) && item.resolved != Some(true))
        .collect();
    let advisory: Vec<_> = packet
        .candidate_actions
        .iter()
        .filter(|item| item.blocking != Some(true) && item.resolved != Some(true))
        .collect();
    out.push_str(&format!(
        "\nWORKLIST ({} blocking, {} advisory, {} resolved)\n",
        blocking.len(),
        advisory.len(),
        packet
            .candidate_actions
            .iter()
            .filter(|item| item.resolved == Some(true))
            .count()
    ));
    for item in blocking.iter().chain(advisory.iter()) {
        let flag = if item.blocking == Some(true) {
            "BLOCKING"
        } else {
            "advisory"
        };
        out.push_str(&format!(
            "  [{flag}][{}] {} - {}\n",
            item.category.as_deref().unwrap_or("?"),
            item.id,
            truncate(&item.title, 90)
        ));
        if item.blocking == Some(true) {
            if let Some(target) = item.decision_dependency_ids.first() {
                out.push_str(&format!(
                    "      clear: receipts resolve --run-dir <run-dir> --target {target} --reason \"...\"\n"
                ));
            }
        } else if !item.suggested_argv.is_empty() {
            out.push_str(&format!("      try: {}\n", item.suggested_argv.join(" ")));
        }
    }

    let refutations: Vec<_> = packet
        .contradictions
        .iter()
        .filter(|c| c.id.starts_with("con:receipt:"))
        .collect();
    if !refutations.is_empty() {
        out.push_str(&format!("\nREFUTATIONS ({})\n", refutations.len()));
        for refutation in refutations {
            out.push_str(&format!("  {}\n", truncate(&refutation.summary, 140)));
        }
    }

    let attested = packet
        .trusted_facts
        .iter()
        .filter(|f| f.attestation.as_deref() == Some("attested"))
        .count();
    out.push_str(&format!(
        "\nTRUSTED FACTS ({attested} attested, {} verifier)\n",
        packet.trusted_facts.len() - attested
    ));
    for fact in &packet.trusted_facts {
        out.push_str(&format!(
            "  [{}] {}\n",
            fact.attestation.as_deref().unwrap_or("?"),
            truncate(&fact.statement, 110)
        ));
    }

    if !packet.lane_digests.is_empty() {
        out.push_str("\nLANES\n");
        for digest in &packet.lane_digests {
            out.push_str(&format!(
                "  {} [{}] {} records ({} attested / {} verifier / {} asserted, {} warnings, {} contradictions)\n",
                digest.lane,
                digest.read_recommendation,
                digest.records,
                digest.attested,
                digest.verifier,
                digest.asserted,
                digest.warnings,
                digest.contradictions
            ));
            if !digest.drill_down.is_empty() {
                out.push_str(&format!(
                    "      drill: {}\n",
                    digest.drill_down[..digest.drill_down.len().min(4)].join(", ")
                ));
            }
        }
    }

    if !receipts.is_empty() {
        // Failed checks with no passing successor come FIRST. Field
        // pattern (2026-07-13): a failed `test:cmd` got "fixed" by minting
        // a fresh label instead of re-running the same one, so the red
        // receipt sat invisible at the bottom of the list. Supersession
        // only works per-label; Prime has to SEE the labels still red.
        let mut latest_by_label: std::collections::BTreeMap<&str, &crate::schema::ReceiptRecord> =
            std::collections::BTreeMap::new();
        for receipt in &receipts {
            if let Some(label) = receipt.label.as_deref() {
                if label != crate::compiler::receipts::WORK_LABEL {
                    latest_by_label.insert(label, receipt);
                }
            }
        }
        let failing: Vec<_> = latest_by_label
            .values()
            .filter(|receipt| receipt.exit_code != 0)
            .collect();
        if !failing.is_empty() {
            out.push_str(&format!(
                "\nFAILED CHECKS ({}) - latest receipt red, no passing successor; re-run the SAME label to supersede\n",
                failing.len()
            ));
            for receipt in failing {
                out.push_str(&format!(
                    "  {} [{}] exit {} - {}\n",
                    receipt.id,
                    receipt.label.as_deref().unwrap_or("-"),
                    receipt.exit_code,
                    truncate(&receipt.cmd.join(" "), 70)
                ));
            }
        }
        out.push_str(&format!("\nRECEIPTS ({})\n", receipts.len()));
        for receipt in &receipts {
            out.push_str(&format!(
                "  {} [{}] exit {} {}ms - {}\n",
                receipt.id,
                receipt.label.as_deref().unwrap_or("-"),
                receipt.exit_code,
                receipt.duration_ms,
                truncate(&receipt.cmd.join(" "), 70)
            ));
        }
    }

    out.push_str(&match (gate.is_some(), drift_warnings.len()) {
        (false, _) => "\nDRIFT: unknown - run receipts gate\n".to_string(),
        (true, 0) => "\nDRIFT: none recorded\n".to_string(),
        (true, count) => format!(
            "\nDRIFT: {count} citation(s) drifted since review (post-review fixes are the usual cause)\n"
        ),
    });

    Ok(out)
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let cut: String = text.chars().take(max).collect();
        format!("{cut}...")
    }
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("abcdefghijk", 5), "abcde...");
    }
}
