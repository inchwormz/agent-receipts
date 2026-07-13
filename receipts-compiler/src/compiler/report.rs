//! `receipts report`: renders `<run-dir>/state/report.html`, a single
//! self-contained HTML page a non-technical founder can read to answer
//! "what did the agents actually do, and how much of it is proven?"
//!
//! `render_report` is pure (no system-time calls, fully unit-testable); its
//! header timestamp is derived deterministically from the latest `ended_at`
//! / `observed_at` already present in the receipts and packet it was handed.
//! `generate_report` does the I/O: load the compiled packet, the verified
//! receipt journal, and an optional gate report, then write the page.

use crate::compiler::receipts::load_verified_receipts;
use crate::schema::{Contradiction, EvidenceRecord, NextPassPacket, ReceiptRecord};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Shape of `state/gate-report.json`, written by the JS gate runtime (not
/// this compiler). Deliberately loose: missing fields default, extra fields
/// are ignored by serde's default (non-`deny_unknown_fields`) behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateReport {
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub errors: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Escape the five HTML-significant characters. Every summary, statement,
/// id, label, command, and error string that reaches the page MUST pass
/// through this — none of it has been sanitized upstream, and all of it may
/// have been authored by an agent.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

fn badge(class: &str, text: &str) -> String {
    format!("<span class=\"badge {class}\">{}</span>", html_escape(text))
}

/// Evidence kinds that never count toward the "asserted" (unverified claim)
/// bucket in the attestation scorecard — infrastructure/quarantined kinds,
/// and "receipt" because runtime receipts are attested by construction.
const EXCLUDED_ASSERTED_KINDS: &[&str] = &[
    "objective",
    "subagent-session",
    "codex-synthesis",
    "blocker",
    "unstructured",
    "receipt",
];

/// THE single definition of an "asserted" record - substantive kind, not
/// promoted to a trusted fact. Shared by the scorecard and the per-row badge
/// so the term means exactly one thing everywhere on the page (critic lane
/// ruling, 2026-07-13: two hand-synced copies had already diverged on the
/// kind exclusion).
fn is_asserted(record: &EvidenceRecord, fact_ids: &BTreeSet<&str>) -> bool {
    !EXCLUDED_ASSERTED_KINDS.contains(&record.kind.as_str())
        && !fact_ids.contains(format!("fact:{}", record.id).as_str())
}

/// Kinds whose claims carry engineering weight even when harvested from
/// prose - an unproven "I changed the code" stays an asserted claim no
/// matter how it arrived. Mirrors the worklist's verify-claim category.
const LOAD_BEARING_KINDS: &[&str] = &["code-change", "test-change", "root-cause"];

/// Field ruling (2026-07-13 loop field run): a prose sentence harvested
/// because it mentioned a file is a NARRATIVE INDEX ENTRY, not an unproven
/// claim - counting it against attestation coverage is a category error
/// (the run's 17 "asserted" records were all this class, drowning the real
/// signal of 0 unproven load-bearing claims). Narrative = asserted-tier,
/// harvested, and not a load-bearing kind. Never promoted, never trusted -
/// just counted honestly as what it is.
fn is_narrative(record: &EvidenceRecord, fact_ids: &BTreeSet<&str>) -> bool {
    is_asserted(record, fact_ids)
        && record.rationale.as_deref() == Some("harvested-from-prose")
        && !LOAD_BEARING_KINDS.contains(&record.kind.as_str())
}

/// Latest timestamp found across the verified receipts and the packet's
/// evidence records, compared lexicographically (ISO-8601 strings sort
/// correctly that way). `None` when there is nothing to anchor on.
fn latest_timestamp(packet: &NextPassPacket, receipts: &[ReceiptRecord]) -> Option<String> {
    let mut latest: Option<&str> = None;
    for receipt in receipts {
        if latest.is_none_or(|cur| receipt.ended_at.as_str() > cur) {
            latest = Some(receipt.ended_at.as_str());
        }
    }
    for record in &packet.evidence {
        if latest.is_none_or(|cur| record.observed_at.as_str() > cur) {
            latest = Some(record.observed_at.as_str());
        }
    }
    latest.map(str::to_string)
}

fn render_header(packet: &NextPassPacket, receipts: &[ReceiptRecord]) -> String {
    let mut out = String::new();
    out.push_str("<header>\n");
    out.push_str(&format!("<h1>{}</h1>\n", html_escape(&packet.objective)));
    out.push_str("<p class=\"meta\">");
    out.push_str(&format!(
        "run <code>{}</code> - pass <code>{}</code>",
        html_escape(&packet.run_id),
        html_escape(&packet.pass_id)
    ));
    if let Some(ts) = latest_timestamp(packet, receipts) {
        out.push_str(&format!(" - generated {}", html_escape(&ts)));
    }
    out.push_str("</p>\n</header>\n");
    out
}

fn render_verdict_banner(gate: Option<&GateReport>) -> String {
    match gate {
        Some(g) if g.ok => "<section class=\"verdict verdict-pass\">GATE PASSED</section>\n".to_string(),
        Some(g) => {
            let mut out =
                String::from("<section class=\"verdict verdict-fail\">\n<p>GATE FAILED</p>\n<ul>\n");
            for err in &g.errors {
                out.push_str(&format!("<li>{}</li>\n", html_escape(err)));
            }
            out.push_str("</ul>\n</section>\n");
            out
        }
        None => "<section class=\"verdict verdict-unknown\">GATE NOT RECORDED - run receipts gate</section>\n"
            .to_string(),
    }
}

fn compute_scorecard(packet: &NextPassPacket) -> (usize, usize, usize, usize, usize, usize) {
    let attested = packet
        .trusted_facts
        .iter()
        .filter(|f| f.attestation.as_deref() == Some("attested"))
        .count();
    let verifier = packet
        .trusted_facts
        .iter()
        .filter(|f| f.attestation.as_deref() == Some("verifier"))
        .count();
    let refuted = packet
        .contradictions
        .iter()
        .filter(|c| c.id.starts_with("con:receipt:"))
        .count();
    let fact_ids: BTreeSet<&str> = packet.trusted_facts.iter().map(|f| f.id.as_str()).collect();
    let narrative = packet
        .evidence
        .iter()
        .filter(|e| is_narrative(e, &fact_ids))
        .count();
    let asserted = packet
        .evidence
        .iter()
        .filter(|e| is_asserted(e, &fact_ids) && !is_narrative(e, &fact_ids))
        .count();
    let unstructured = packet
        .evidence
        .iter()
        .filter(|e| e.kind == "unstructured")
        .count();
    (
        attested,
        verifier,
        refuted,
        asserted,
        narrative,
        unstructured,
    )
}

fn render_scorecard(packet: &NextPassPacket) -> String {
    let (attested, verifier, refuted, asserted, narrative, unstructured) =
        compute_scorecard(packet);
    let denom = attested + verifier + asserted;
    let coverage_text = if denom == 0 {
        "no substantive claims".to_string()
    } else {
        let pct = ((attested as f64 / denom as f64) * 100.0).round() as i64;
        format!("{pct}% attested")
    };

    format!(
        "<section class=\"scorecard\">\n<h2>Attestation scorecard</h2>\n\
         <p class=\"coverage\">{coverage} <span class=\"raw\">({attested} attested, {verifier} verifier, {asserted} asserted; {narrative} narrative lines indexed)</span></p>\n\
         <ul class=\"scorecard-counts\">\n\
         <li>Attested: {attested}</li>\n\
         <li>Verifier: {verifier}</li>\n\
         <li>Asserted: {asserted}</li>\n\
         <li>Narrative: {narrative}</li>\n\
         <li>Refuted: {refuted}</li>\n\
         <li>Unstructured: {unstructured}</li>\n\
         </ul>\n</section>\n",
        coverage = coverage_text,
    )
}

fn render_refutations(packet: &NextPassPacket) -> String {
    let refutations: Vec<&Contradiction> = packet
        .contradictions
        .iter()
        .filter(|c| c.id.starts_with("con:receipt:"))
        .collect();
    if refutations.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"refutations\">\n<h2>Refutations</h2>\n<ul>\n");
    for c in refutations {
        out.push_str(&format!(
            "<li><strong>{}</strong>: {}</li>\n",
            html_escape(&c.id),
            html_escape(&c.summary)
        ));
    }
    out.push_str("</ul>\n</section>\n");
    out
}

/// Phase 3: the worklist - what Prime should DO, blocking first. Resolved
/// items stay visible (struck through) for audit.
fn render_worklist(packet: &NextPassPacket) -> String {
    if packet.candidate_actions.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"worklist\">\n<h2>Worklist</h2>\n<ul>\n");
    for item in &packet.candidate_actions {
        let class = if item.resolved == Some(true) {
            "resolved"
        } else if item.blocking == Some(true) {
            "blocking"
        } else {
            "advisory"
        };
        let badge_html = if item.blocking == Some(true) {
            badge("badge-red", "blocking")
        } else {
            badge("badge-grey", "advisory")
        };
        let resolution = item
            .resolution_id
            .as_deref()
            .map(|id| {
                format!(
                    " <span class=\"raw\">resolved by {}</span>",
                    html_escape(id)
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<li class=\"{class}\">{} <span class=\"kind\">{}</span> <code>{}</code> - {}{}</li>\n",
            badge_html,
            html_escape(item.category.as_deref().unwrap_or("action")),
            html_escape(&item.id),
            html_escape(&item.title),
            resolution
        ));
    }
    out.push_str("</ul>\n</section>\n");
    out
}

/// Phase 3: lane digests - which lanes to read, with drill-down handles.
fn render_lane_digests(packet: &NextPassPacket) -> String {
    if packet.lane_digests.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"digests\">\n<h2>Lane digests</h2>\n<ul>\n");
    for digest in &packet.lane_digests {
        let badge_class = match digest.read_recommendation.as_str() {
            "skip-verified" => "badge-green",
            "blocked" => "badge-red",
            "read-adjudicate" => "badge-amber",
            _ => "badge-grey",
        };
        let drill = if digest.drill_down.is_empty() {
            String::new()
        } else {
            format!(
                " <span class=\"raw\">drill: {}</span>",
                html_escape(&digest.drill_down[..digest.drill_down.len().min(4)].join(", "))
            )
        };
        out.push_str(&format!(
            "<li><strong>{}</strong> {} {} records ({} attested / {} verifier / {} asserted, {} warnings, {} contradictions){}</li>\n",
            html_escape(&digest.lane),
            badge(badge_class, &digest.read_recommendation),
            digest.records,
            digest.attested,
            digest.verifier,
            digest.asserted,
            digest.warnings,
            digest.contradictions,
            drill
        ));
    }
    out.push_str("</ul>\n</section>\n");
    out
}

/// Phase 1: "what changed on disk" - work receipts rendered as a scope
/// section. Notes are agent/Prime prose and are labeled asserted.
fn render_work_scope(packet: &NextPassPacket) -> String {
    let work: Vec<&EvidenceRecord> = packet
        .evidence
        .iter()
        .filter(|record| record.kind == "work")
        .collect();
    if work.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section class=\"work-scope\">\n<h2>Codebase changes</h2>\n<ul>\n");
    for record in work {
        let note = record
            .rationale
            .as_deref()
            .map(|text| {
                format!(
                    " <span class=\"raw\">note (asserted): {}</span>",
                    html_escape(text)
                )
            })
            .unwrap_or_default();
        out.push_str(&format!(
            "<li><code>{}</code> {}{}</li>\n",
            html_escape(&record.id),
            html_escape(&record.summary),
            note
        ));
    }
    out.push_str("</ul>\n</section>\n");
    out
}

fn render_receipts_table(receipts: &[ReceiptRecord]) -> String {
    let mut out = String::from("<section class=\"receipts\">\n<h2>Receipts</h2>\n");
    if receipts.is_empty() {
        out.push_str("<p class=\"empty\">No execution receipts recorded.</p>\n</section>\n");
        return out;
    }
    out.push_str(
        "<table>\n<thead><tr><th>id</th><th>label</th><th>command</th><th>exit</th><th>duration_ms</th><th>output</th></tr></thead>\n<tbody>\n",
    );
    for r in receipts {
        let label = r.label.as_deref().unwrap_or("-");
        let exit_class = if r.exit_code == 0 {
            "exit-ok"
        } else {
            "exit-fail"
        };
        let cmd = r.cmd.join(" ");
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td><code>{}</code></td><td class=\"{}\">{}</td><td>{}</td><td><a href=\"../receipts/artifacts/{}.txt\">stdout</a> / <a href=\"../receipts/artifacts/{}.txt\">stderr</a></td></tr>\n",
            html_escape(&r.id),
            html_escape(label),
            html_escape(&cmd),
            exit_class,
            r.exit_code,
            r.duration_ms,
            html_escape(&r.stdout_hash),
            html_escape(&r.stderr_hash),
        ));
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
    out
}

fn render_trusted_facts(packet: &NextPassPacket) -> String {
    let mut out = String::from("<section class=\"facts\">\n<h2>Trusted facts</h2>\n");
    if packet.trusted_facts.is_empty() {
        out.push_str("<p class=\"empty\">No trusted facts yet.</p>\n</section>\n");
        return out;
    }
    out.push_str(
        "<table>\n<thead><tr><th>tier</th><th>statement</th><th>source_ids</th></tr></thead>\n<tbody>\n",
    );
    for f in &packet.trusted_facts {
        let (badge_class, badge_text) = match f.attestation.as_deref() {
            Some("attested") => ("badge-green", "attested"),
            Some("verifier") => ("badge-blue", "verifier"),
            other => ("badge-grey", other.unwrap_or("unknown")),
        };
        out.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            badge(badge_class, badge_text),
            html_escape(&f.statement),
            html_escape(&f.source_ids.join(", "))
        ));
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
    out
}

fn render_evidence_by_lane(packet: &NextPassPacket) -> String {
    let fact_ids: BTreeSet<&str> = packet.trusted_facts.iter().map(|f| f.id.as_str()).collect();
    let mut lanes: Vec<(String, Vec<&EvidenceRecord>)> = Vec::new();
    for e in &packet.evidence {
        let lane = e.lane.clone().unwrap_or_else(|| "(no lane)".to_string());
        match lanes.iter_mut().find(|(name, _)| *name == lane) {
            Some(entry) => entry.1.push(e),
            None => lanes.push((lane, vec![e])),
        }
    }

    let mut out = String::from("<section class=\"evidence\">\n<h2>Evidence by lane</h2>\n");
    if lanes.is_empty() {
        out.push_str("<p class=\"empty\">No evidence recorded.</p>\n</section>\n");
        return out;
    }
    for (lane, records) in lanes {
        out.push_str(&format!(
            "<h3>{}</h3>\n<ul class=\"evidence-lane\">\n",
            html_escape(&lane)
        ));
        for e in records {
            let mut badges = String::new();
            if is_narrative(e, &fact_ids) {
                badges.push_str(&badge("badge-slate", "narrative"));
            } else if is_asserted(e, &fact_ids) {
                badges.push_str(&badge("badge-grey", "asserted"));
            }
            let mut warnings_html = String::new();
            if !e.provenance_warnings.is_empty() {
                badges.push_str(&badge("badge-orange", "demoted"));
                warnings_html.push_str("<ul class=\"warnings\">\n");
                for w in &e.provenance_warnings {
                    warnings_html.push_str(&format!("<li>{}</li>\n", html_escape(w)));
                }
                warnings_html.push_str("</ul>\n");
            }
            if e.kind == "unstructured" {
                badges.push_str(&badge("badge-purple", "unstructured"));
            }
            out.push_str(&format!(
                "<li><code>{}</code> <span class=\"kind\">{}</span> {} - {}{}</li>\n",
                html_escape(&e.id),
                html_escape(&e.kind),
                badges,
                html_escape(&e.summary),
                warnings_html
            ));
        }
        out.push_str("</ul>\n");
    }
    out.push_str("</section>\n");
    out
}

fn render_verifier_findings(packet: &NextPassPacket) -> String {
    let mut out =
        String::from("<section class=\"verifier-findings\">\n<h2>Verifier findings</h2>\n");
    if packet.verifier_findings.is_empty() {
        out.push_str("<p class=\"empty\">No verifier findings recorded.</p>\n</section>\n");
        return out;
    }
    out.push_str(
        "<table>\n<thead><tr><th>id</th><th>status</th><th>score</th><th>summary</th><th>finding_kind</th></tr></thead>\n<tbody>\n",
    );
    for f in &packet.verifier_findings {
        let status_class = match f.status.as_str() {
            "passed" => "status-pass",
            "failed" => "status-fail",
            _ => "status-other",
        };
        let finding_kind = f.finding_kind.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "<tr><td>{}</td><td class=\"{}\">{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            html_escape(&f.id),
            status_class,
            html_escape(&f.status),
            f.verifier_score,
            html_escape(&f.summary),
            html_escape(finding_kind)
        ));
    }
    out.push_str("</tbody>\n</table>\n</section>\n");
    out
}

const FOOTER: &str = "<footer><p>Generated by receipts report - Receipts engine</p></footer>\n";

const STYLE: &str = r#"<style>
:root { color-scheme: light; }
* { box-sizing: border-box; }
body {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    background: #f7f7f8;
    color: #1a1a1a;
}
main {
    max-width: 960px;
    margin: 0 auto;
    padding: 2rem 1.5rem 4rem;
}
h1 { font-size: 1.6rem; margin-bottom: 0.25rem; }
h2 { font-size: 1.2rem; margin-top: 2rem; border-bottom: 1px solid #ddd; padding-bottom: 0.25rem; }
h3 { font-size: 1rem; margin-top: 1.25rem; }
.meta { color: #555; font-size: 0.9rem; }
.meta code { background: #eee; padding: 0 0.3rem; border-radius: 3px; }
section { margin-top: 1rem; }
.empty { color: #777; font-style: italic; }
.verdict {
    padding: 0.75rem 1rem;
    border-radius: 6px;
    font-weight: 600;
    margin: 1rem 0;
}
.verdict-pass { background: #e3f5e6; color: #1c6b2c; }
.verdict-fail { background: #fbe4e4; color: #a11c1c; }
.verdict-unknown { background: #fdf3d8; color: #8a6a00; }
.verdict-fail ul { margin: 0.5rem 0 0; padding-left: 1.25rem; }
.scorecard .coverage { font-size: 1.1rem; font-weight: 600; }
.scorecard .raw { font-weight: 400; color: #555; font-size: 0.9rem; }
.scorecard-counts { list-style: none; padding: 0; display: flex; flex-wrap: wrap; gap: 1rem; color: #444; }
table { width: 100%; border-collapse: collapse; margin-top: 0.5rem; font-size: 0.9rem; }
th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #e2e2e2; vertical-align: top; }
tbody tr:nth-child(even) { background: #f0f0f1; }
.exit-ok { color: #1c6b2c; font-weight: 600; }
.exit-fail { color: #a12622; font-weight: 700; background: #fbeceb; }
.worklist li.blocking { border-left: 3px solid #a12622; padding-left: 8px; }
.worklist li.advisory { border-left: 3px solid #b8b8b8; padding-left: 8px; }
.worklist li.resolved { opacity: 0.55; text-decoration: line-through; }
.badge-red { background: #fbeceb; color: #a12622; }
.badge-amber { background: #fdf3e0; color: #8a5b00; }
.exit-fail { color: #a11c1c; font-weight: 600; }
.status-pass { color: #1c6b2c; font-weight: 600; }
.status-fail { color: #a11c1c; font-weight: 600; }
.status-other { color: #8a6a00; font-weight: 600; }
.badge {
    display: inline-block;
    border-radius: 999px;
    padding: 0.1rem 0.55rem;
    font-size: 0.75rem;
    font-weight: 600;
    margin-right: 0.3rem;
}
.badge-green { background: #dff3e2; color: #1c6b2c; }
.badge-blue { background: #dde8fb; color: #1c4b9c; }
.badge-grey { background: #e8e8e8; color: #444; }
.badge-slate { background: #e4e9f0; color: #3d4d63; }
.badge-orange { background: #fbe8d2; color: #9c5a00; }
.badge-purple { background: #ece0f8; color: #5c1c9c; }
.refutations { background: #fbe4e4; padding: 1rem; border-radius: 6px; }
.refutations ul { margin: 0.5rem 0 0; padding-left: 1.25rem; }
.evidence-lane { list-style: none; padding: 0; }
.evidence-lane li { padding: 0.3rem 0; border-bottom: 1px solid #eee; }
.warnings { margin: 0.25rem 0 0 1.5rem; color: #9c5a00; font-size: 0.85rem; }
footer { margin-top: 3rem; color: #888; font-size: 0.85rem; text-align: center; }
</style>
"#;

/// Pure render: no system-time calls, no I/O. Everything shown is derived
/// from `packet`, `receipts`, and `gate`.
pub fn render_report(
    packet: &NextPassPacket,
    receipts: &[ReceiptRecord],
    gate: Option<&GateReport>,
) -> String {
    let mut html = String::new();
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    html.push_str(&format!(
        "<title>receipts report - {}</title>\n",
        html_escape(&packet.run_id)
    ));
    html.push_str(STYLE);
    html.push_str("</head>\n<body>\n<main>\n");

    html.push_str(&render_header(packet, receipts));
    html.push_str(&render_verdict_banner(gate));
    html.push_str(&render_scorecard(packet));
    html.push_str(&render_refutations(packet));
    html.push_str(&render_worklist(packet));
    html.push_str(&render_work_scope(packet));
    html.push_str(&render_receipts_table(receipts));
    html.push_str(&render_trusted_facts(packet));
    html.push_str(&render_lane_digests(packet));
    html.push_str(&render_evidence_by_lane(packet));
    html.push_str(&render_verifier_findings(packet));
    html.push_str(FOOTER);

    html.push_str("</main>\n</body>\n</html>\n");
    html
}

/// Load `state/next_pass_packet.json`, the verified receipt journal, and an
/// optional `state/gate-report.json`, render the page, and write
/// `state/report.html`. Returns the written path.
pub fn generate_report(run_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let state_dir = run_dir.join("state");
    let packet_path = state_dir.join("next_pass_packet.json");
    let packet_text = fs::read_to_string(&packet_path).map_err(|err| {
        format!(
            "failed to read {}: {err} - run `receipts compile --run-dir <dir>` first",
            packet_path.display()
        )
    })?;
    let packet: NextPassPacket = serde_json::from_str(&packet_text).map_err(|err| {
        format!(
            "{} is not a valid next-pass packet: {err}",
            packet_path.display()
        )
    })?;

    let receipts = load_verified_receipts(run_dir)?;

    let gate_path = state_dir.join("gate-report.json");
    let gate: Option<GateReport> =
        if gate_path.exists() {
            let text = fs::read_to_string(&gate_path)?;
            Some(serde_json::from_str(&text).map_err(|err| {
                format!("{} is not a valid gate report: {err}", gate_path.display())
            })?)
        } else {
            None
        };

    let html = render_report(&packet, &receipts, gate.as_ref());

    fs::create_dir_all(&state_dir)?;
    let report_path = state_dir.join("report.html");
    fs::write(&report_path, html)?;
    Ok(report_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CompiledFact, RECEIPTS_SCHEMA_VERSION};

    fn empty_packet() -> NextPassPacket {
        NextPassPacket {
            schema_version: RECEIPTS_SCHEMA_VERSION.to_string(),
            objective_id: "obj-1".to_string(),
            run_id: "run-1".to_string(),
            branch_id: "main".to_string(),
            pass_id: "pass-1".to_string(),
            objective: "Test objective".to_string(),
            evidence: vec![],
            trusted_facts: vec![],
            active_hypotheses: vec![],
            contradictions: vec![],
            recurring_failure_patterns: vec![],
            candidate_actions: vec![],
            verifier_findings: vec![],
            open_questions: vec![],
            raw_drilldown_refs: vec![],
            halt_signals: vec![],
            sources: vec![],
            lane_digests: vec![],
        }
    }

    fn evidence(id: &str, kind: &str, summary: &str) -> EvidenceRecord {
        EvidenceRecord {
            id: id.to_string(),
            kind: kind.to_string(),
            summary: summary.to_string(),
            source_ids: vec![],
            source_refs: vec![],
            observed_at: "2026-07-12T00:00:00Z".to_string(),
            agent_id: None,
            lane: None,
            confidence: None,
            rationale: None,
            diff_ref: None,
            span_before: None,
            span_after: None,
            claimed_agent_id: None,
            claimed_lane: None,
            provenance_warnings: vec![],
        }
    }

    fn fact(id: &str, attestation: &str) -> CompiledFact {
        CompiledFact {
            id: id.to_string(),
            statement: "Some statement".to_string(),
            confidence: 0.8,
            objective_relevance: 0.8,
            novelty_gain: 0.3,
            needs_raw_drilldown: false,
            source_ids: vec![],
            attestation: Some(attestation.to_string()),
        }
    }

    fn receipt(id: &str, label: Option<&str>, exit_code: i64) -> ReceiptRecord {
        ReceiptRecord {
            id: id.to_string(),
            label: label.map(str::to_string),
            cmd: vec!["cargo".to_string(), "test".to_string()],
            cwd: ".".to_string(),
            exit_code,
            duration_ms: 42,
            started_at: "2026-07-12T00:00:00Z".to_string(),
            ended_at: "2026-07-12T00:00:01Z".to_string(),
            stdout_hash: "aaaaaaaaaaaaaaaa".to_string(),
            stderr_hash: "bbbbbbbbbbbbbbbb".to_string(),
            stdout_tail: String::new(),
            stderr_tail: String::new(),
            tree_before: None,
            tree_after: None,
            lane: None,
            agent_id: None,
            writer: "receipts-test".to_string(),
            prev_record_hash: "GENESIS".to_string(),
            record_hash: "cccccccccccccccc".to_string(),
        }
    }

    #[test]
    fn html_escape_escapes_all_five_characters() {
        assert_eq!(html_escape("&<>\"'"), "&amp;&lt;&gt;&quot;&#39;");
    }

    #[test]
    fn harvested_prose_is_narrative_not_asserted_unless_load_bearing() {
        let mut packet = empty_packet();
        let mut mention = evidence("ev-n1", "observation", "the dispatcher lives in bin/x.mjs");
        mention.rationale = Some("harvested-from-prose".to_string());
        let mut load_bearing = evidence("ev-n2", "code-change", "I rewrote bin/x.mjs entirely");
        load_bearing.rationale = Some("harvested-from-prose".to_string());
        packet.evidence.push(mention);
        packet.evidence.push(load_bearing);

        let html = render_report(&packet, &[], None);
        // The harvested mention is narrative: out of the asserted bucket and
        // out of the coverage denominator; the harvested code-change claim
        // stays asserted - "I changed the code" is load-bearing however it
        // arrived.
        assert!(
            html.contains("1 asserted; 1 narrative lines indexed"),
            "expected 1 asserted + 1 narrative: {html}"
        );
        assert!(
            html.contains(">narrative</span>"),
            "narrative badge missing: {html}"
        );
        let asserted_badges = html.matches(">asserted</span>").count();
        assert_eq!(
            asserted_badges, 1,
            "only the load-bearing harvested claim wears the asserted badge: {html}"
        );
    }

    #[test]
    fn asserted_means_one_thing_everywhere_on_the_page() {
        // Critic-lane ruling regression: the scorecard and the per-row badge
        // must share the SAME definition of "asserted". Infrastructure kinds
        // (objective) and quarantined prose (unstructured) never get the
        // asserted badge even though they are unpromoted.
        let mut packet = empty_packet();
        packet
            .evidence
            .push(evidence("ev-objective", "objective", "the objective"));
        packet
            .evidence
            .push(evidence("ev-prose", "unstructured", "quarantined prose"));
        packet.evidence.push(evidence(
            "ev-real-claim",
            "observation",
            "an unproven claim",
        ));
        let html = render_report(&packet, &[], None);

        // Scorecard counts exactly one asserted record...
        assert!(
            html.contains("1 asserted"),
            "scorecard must count exactly the substantive unpromoted record: {html}"
        );
        // ...and exactly one asserted badge is rendered (the observation's).
        let badge_count = html.matches(">asserted</span>").count();
        assert_eq!(
            badge_count, 1,
            "exactly one asserted badge (the substantive claim), got {badge_count}"
        );
        // The unstructured row keeps its own badge without the contradictory
        // asserted badge alongside.
        assert!(html.contains(">unstructured</span>"));
    }

    #[test]
    fn xss_summary_is_escaped_and_raw_script_tag_never_appears() {
        let mut packet = empty_packet();
        packet.evidence.push(evidence(
            "ev-xss",
            "observation",
            "<script>alert(1)</script>",
        ));
        let html = render_report(&packet, &[], None);
        assert!(
            html.contains("&lt;script&gt;"),
            "expected escaped script tag in output"
        );
        assert!(
            !html.contains("<script"),
            "raw <script sequence must never appear: {html}"
        );
    }

    #[test]
    fn con_receipt_contradiction_renders_in_refutations_panel() {
        let mut packet = empty_packet();
        packet.contradictions.push(Contradiction {
            id: "con:receipt:vf-1:rcpt-0001".to_string(),
            summary: "claim refuted by execution".to_string(),
            conflicting_item_ids: vec!["vf-1".to_string()],
            severity: "high".to_string(),
            source_ids: vec![],
            source_refs: None,
        });
        let html = render_report(&packet, &[], None);
        assert!(
            html.contains("class=\"refutations\""),
            "refutations panel must render"
        );
        assert!(html.contains("claim refuted by execution"));
    }

    #[test]
    fn receipt_row_renders_exit_code_and_label() {
        let r = receipt("rcpt-0001", Some("test:cargo-suite"), 0);
        let html = render_report(&empty_packet(), &[r], None);
        assert!(
            html.contains("test:cargo-suite"),
            "label must render: {html}"
        );
        assert!(html.contains(">0<"), "exit code 0 must render: {html}");
    }

    #[test]
    fn coverage_math_two_attested_one_verifier_one_asserted() {
        let mut packet = empty_packet();
        packet
            .evidence
            .push(evidence("ev-1", "observation", "asserted claim"));
        packet.trusted_facts.push(fact("fact:ev-2", "attested"));
        packet.trusted_facts.push(fact("fact:ev-3", "attested"));
        packet.trusted_facts.push(fact("fact:ev-4", "verifier"));
        let html = render_report(&packet, &[], None);
        assert!(
            html.contains("50% attested"),
            "expected 50% coverage: {html}"
        );
    }
}
