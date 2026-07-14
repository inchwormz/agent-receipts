//! Consent-gated public reliability data and deterministic static cards.
//!
//! Publication constructs a fixed allowlist from a freshly recomputed score.
//! Private packets, source text, command output, repository locations, and
//! consent metadata are never copied into public data.

use crate::compiler::crypto::{ExecutorIdentity, hex_decode, sign_detached, verify_detached};
use crate::compiler::report::html_escape;
use crate::compiler::run_dir::RunManifest;
use crate::compiler::scoring::{CompletionInterval, ReliabilityScore, ScoreVersions, score_run};
use crate::schema::EvidenceCoverage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const RECORD_HASH_DOMAIN: &[u8] = b"agent-receipts:public-reliability-card:v1:hash";
const RECORD_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:public-reliability-card:v1:signature";
const CARD_GENERATOR_VERSION: &str = "1.0.0";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicationConsent {
    format_version: String,
    consent: bool,
    run_id: String,
    calibration_bundle: String,
    public_data_version: String,
    license: String,
    authorized_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PublicReliabilityMetrics {
    pub score_status: String,
    pub calibration_status: String,
    pub task_family: String,
    pub false_green_probability: Option<f64>,
    pub upper_95_false_green_risk: Option<f64>,
    pub false_green_interval_95_width: Option<f64>,
    pub verified_completion: Option<CompletionInterval>,
    pub held_out_calibration_passed: bool,
    pub categorical_suppression: bool,
    pub out_of_domain_warnings: Vec<String>,
    pub evidence_coverage: Option<EvidenceCoverage>,
    pub critical_claims: u64,
    pub bound_critical_claims: u64,
    pub first_pass_success_rate: Option<f64>,
    pub mean_attempts_to_green: Option<f64>,
    pub flake_rate: Option<f64>,
    pub human_escalation_rate: Option<f64>,
    pub cost_usd: Option<f64>,
    pub elapsed_ms: Option<u64>,
    pub raw_sample_size: u64,
    pub effective_sample_size: f64,
    pub versions: ScoreVersions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PublicReliabilityCard {
    pub format_version: String,
    pub record_kind: String,
    pub public_data_version: String,
    pub public_id: String,
    pub data_license: String,
    pub metrics: PublicReliabilityMetrics,
    pub executor: ExecutorIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub record_hash: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
struct StaticCards<'a> {
    format_version: &'static str,
    card_generator_version: &'static str,
    data_license: &'static str,
    cards: &'a [PublicReliabilityCard],
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn safe_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn secret_marker(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    [
        ("github_pat_", "GitHub token"),
        ("ghp_", "GitHub token"),
        ("gho_", "GitHub token"),
        ("sk-", "API key"),
        ("akia", "AWS access key"),
        ("bearer ", "bearer token"),
        ("-----begin", "private key"),
        ("password=", "password"),
        ("api_key", "API key"),
        ("api-key", "API key"),
        ("token=", "token"),
    ]
    .into_iter()
    .find_map(|(needle, label)| lower.contains(needle).then_some(label))
}

fn scan_public_text(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(marker) = secret_marker(text) {
        return Err(format!("public reliability data contains a secret marker ({marker})").into());
    }
    let lower = text.to_ascii_lowercase();
    for forbidden in [
        "c:\\",
        "/users/",
        "/home/",
        "repo_root",
        "repository_url",
        "source_text",
        "source_ids",
        "stdout",
        "stderr",
        "prompt",
        "\"cmd\"",
    ] {
        if lower.contains(forbidden) {
            return Err(format!(
                "public reliability data contains forbidden private material `{forbidden}`"
            )
            .into());
        }
    }
    Ok(())
}

fn canonical_unsigned(
    record: &PublicReliabilityCard,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut canonical = record.clone();
    canonical.record_hash.clear();
    canonical.signature.clear();
    Ok(serde_json::to_vec(&canonical)?)
}

fn record_hash(record: &PublicReliabilityCard) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(RECORD_HASH_DOMAIN);
    hasher.update(&canonical_unsigned(record)?);
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn verify_public_card(
    record: &PublicReliabilityCard,
) -> Result<(), Box<dyn std::error::Error>> {
    if record.format_version != "1"
        || record.record_kind != "public_reliability_card"
        || !safe_segment(&record.public_data_version)
        || !safe_segment(&record.public_id)
        || record.data_license != "CC-BY-4.0"
        || record.hash_alg != "blake3-256"
        || record.signature_alg != "ed25519"
        || record.record_hash.len() != 64
        || record.signature.len() != 128
    {
        return Err("public reliability card has unsupported metadata".into());
    }
    let actual = record_hash(record)?;
    if actual != record.record_hash {
        return Err("public reliability card hash mismatch".into());
    }
    verify_detached(
        RECORD_SIGNATURE_DOMAIN,
        &hex_decode(&record.record_hash)?,
        &record.executor,
        &record.signature,
    )?;
    scan_public_text(&String::from_utf8(serde_json::to_vec(record)?)?)
}

fn public_metrics(score: ReliabilityScore) -> PublicReliabilityMetrics {
    PublicReliabilityMetrics {
        score_status: score.score_status,
        calibration_status: score.calibration_status,
        task_family: score.task_family,
        false_green_probability: score.false_green_probability,
        upper_95_false_green_risk: score.upper_95_false_green_risk,
        false_green_interval_95_width: score.false_green_interval_95_width,
        verified_completion: score.verified_completion,
        held_out_calibration_passed: score.held_out_calibration_passed,
        categorical_suppression: !score.suppression_reasons.is_empty(),
        out_of_domain_warnings: score.out_of_domain_warnings,
        evidence_coverage: score.evidence_coverage,
        critical_claims: score.critical_claims,
        bound_critical_claims: score.bound_critical_claims,
        first_pass_success_rate: score.first_pass_success_rate,
        mean_attempts_to_green: score.mean_attempts_to_green,
        flake_rate: score.flake_rate,
        human_escalation_rate: score.human_escalation_rate,
        cost_usd: score.cost_usd,
        elapsed_ms: score.elapsed_ms,
        raw_sample_size: score.raw_sample_size,
        effective_sample_size: score.effective_sample_size,
        versions: score.versions,
    }
}

pub fn publish_run(
    run_dir: &Path,
    consent_path: &Path,
    out_dir: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let consent_bytes = fs::read(consent_path)
        .map_err(|error| format!("explicit publication consent file is required: {error}"))?;
    let consent_text = String::from_utf8(consent_bytes.clone())?;
    if let Some(marker) = secret_marker(&consent_text) {
        return Err(format!("publication consent contains a secret marker ({marker})").into());
    }
    let consent: PublicationConsent = serde_json::from_slice(&consent_bytes)?;
    if consent.format_version != "1"
        || !consent.consent
        || consent.license != "CC-BY-4.0"
        || consent.authorized_by.trim().is_empty()
        || !safe_segment(&consent.public_data_version)
    {
        return Err("publication consent is invalid or not explicitly granted".into());
    }
    let manifest: RunManifest = read_json(&run_dir.join("manifest.json"))?;
    if consent.run_id != manifest.run_id || !safe_segment(&manifest.run_id) {
        return Err("publication consent run_id does not match the current run".into());
    }
    let declared_bundle = PathBuf::from(&consent.calibration_bundle);
    let bundle_path = if declared_bundle.is_absolute() {
        declared_bundle
    } else {
        consent_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(declared_bundle)
    };
    let score = score_run(run_dir, &bundle_path)?;
    let mut record = PublicReliabilityCard {
        format_version: "1".to_string(),
        record_kind: "public_reliability_card".to_string(),
        public_data_version: consent.public_data_version,
        public_id: manifest.run_id,
        data_license: "CC-BY-4.0".to_string(),
        metrics: public_metrics(score),
        executor: crate::compiler::crypto::current_executor_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        record_hash: String::new(),
        signature: String::new(),
    };
    record.record_hash = record_hash(&record)?;
    let (executor, signature) =
        sign_detached(RECORD_SIGNATURE_DOMAIN, &hex_decode(&record.record_hash)?)?;
    if executor != record.executor {
        return Err("executor identity changed while signing public reliability card".into());
    }
    record.signature = signature;
    verify_public_card(&record)?;
    let mut bytes = serde_json::to_vec_pretty(&record)?;
    bytes.push(b'\n');
    scan_public_text(&String::from_utf8(bytes.clone())?)?;
    let path = out_dir
        .join(&record.public_data_version)
        .join(format!("{}.json", record.public_id));
    fs::create_dir_all(path.parent().ok_or("public card output has no parent")?)?;
    fs::write(&path, bytes)?;
    Ok(path)
}

fn collect_json_files(
    root: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut entries = fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!(
                "public data symlink is not allowed: {}",
                entry.path().display()
            )
            .into());
        }
        if file_type.is_dir() {
            collect_json_files(&entry.path(), files)?;
        } else if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
            files.push(entry.path());
        }
    }
    Ok(())
}

pub fn load_verified_public_cards(
    data_dir: &Path,
) -> Result<Vec<PublicReliabilityCard>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    collect_json_files(data_dir, &mut files)?;
    if files.is_empty() {
        return Err("public data directory contains no JSON reliability cards".into());
    }
    let mut cards = Vec::new();
    let mut identities = BTreeSet::new();
    for path in files {
        let bytes = fs::read(&path)?;
        scan_public_text(&String::from_utf8(bytes.clone())?)?;
        let card: PublicReliabilityCard = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "{} is not a public reliability card: {error}",
                path.display()
            )
        })?;
        verify_public_card(&card)?;
        let identity = format!("{}:{}", card.public_data_version, card.public_id);
        if !identities.insert(identity.clone()) {
            return Err(format!("duplicate public reliability card `{identity}`").into());
        }
        cards.push(card);
    }
    cards.sort_by(|left, right| {
        left.public_data_version
            .cmp(&right.public_data_version)
            .then_with(|| left.public_id.cmp(&right.public_id))
    });
    Ok(cards)
}

fn percentage(value: Option<f64>) -> String {
    value.map_or_else(
        || "Not published".to_string(),
        |value| format!("{:.1}%", value * 100.0),
    )
}

fn number(value: Option<f64>) -> String {
    value.map_or_else(
        || "Not available".to_string(),
        |value| format!("{value:.2}"),
    )
}

fn card_html(card: &PublicReliabilityCard, standalone: bool) -> String {
    let metrics = &card.metrics;
    let completion = metrics.verified_completion.as_ref();
    let coverage = metrics.evidence_coverage.as_ref();
    let warnings = if metrics.out_of_domain_warnings.is_empty() {
        "<p class=\"quiet\">No out-of-domain warnings.</p>".to_string()
    } else {
        format!(
            "<ul class=\"warnings\">{}</ul>",
            metrics
                .out_of_domain_warnings
                .iter()
                .map(|warning| format!("<li>{}</li>", html_escape(warning)))
                .collect::<String>()
        )
    };
    let article = format!(
        "<article class=\"card\"><header><p class=\"eyebrow\">{} · {}</p><h2>{}</h2><span class=\"status\">{}</span></header><div class=\"headline\"><section><span>Upper 95% false-green risk</span><strong>{}</strong><small>Probability that a claimed completion is actually a failure</small></section><section><span>Verified completion</span><strong>{}</strong><small>95% interval {}–{}</small></section></div><dl><div><dt>False-green posterior</dt><dd>{}</dd></div><div><dt>First-pass success</dt><dd>{}</dd></div><div><dt>Attempts to green</dt><dd>{}</dd></div><div><dt>Flake rate</dt><dd>{}</dd></div><div><dt>Evidence coverage</dt><dd>{}/{}</dd></div><div><dt>Critical claim binding</dt><dd>{}/{}</dd></div><div><dt>Human escalation</dt><dd>{}</dd></div><div><dt>Cost</dt><dd>{}</dd></div><div><dt>Elapsed</dt><dd>{}</dd></div><div><dt>Samples</dt><dd>{} raw / {:.2} effective</dd></div></dl><section><h3>Calibration and domain</h3>{}</section><section class=\"versions\"><h3>Exact identities</h3><p>Model: {} · Agent: {} {} · Engine: {} at {} · Dataset: {} · Method: {} · Checks: {}</p></section></article>",
        html_escape(&card.public_data_version),
        html_escape(&metrics.task_family),
        html_escape(&card.public_id),
        html_escape(&metrics.score_status),
        percentage(metrics.upper_95_false_green_risk),
        percentage(completion.map(|value| value.rate)),
        percentage(completion.map(|value| value.lower_95)),
        percentage(completion.map(|value| value.upper_95)),
        percentage(metrics.false_green_probability),
        percentage(metrics.first_pass_success_rate),
        number(metrics.mean_attempts_to_green),
        percentage(metrics.flake_rate),
        coverage.map_or(0, |value| {
            value.verified_claims + value.verifier_backed_claims
        }),
        coverage.map_or(0, |value| value.total_claims),
        metrics.bound_critical_claims,
        metrics.critical_claims,
        percentage(metrics.human_escalation_rate),
        metrics.cost_usd.map_or_else(
            || "Not available".to_string(),
            |value| format!("${value:.2}")
        ),
        metrics.elapsed_ms.map_or_else(
            || "Not available".to_string(),
            |value| format!("{:.2}s", value as f64 / 1000.0)
        ),
        metrics.raw_sample_size,
        metrics.effective_sample_size,
        warnings,
        html_escape(
            metrics
                .versions
                .resolved_model_snapshot
                .as_deref()
                .unwrap_or("unresolved")
        ),
        html_escape(
            metrics
                .versions
                .agent_name
                .as_deref()
                .unwrap_or("unresolved")
        ),
        html_escape(
            metrics
                .versions
                .agent_version
                .as_deref()
                .unwrap_or("unresolved")
        ),
        html_escape(&metrics.versions.engine_version),
        html_escape(&metrics.versions.engine_build_commit),
        html_escape(&metrics.versions.dataset_hash),
        html_escape(&metrics.versions.methodology_version),
        html_escape(&metrics.versions.check_versions.join(", ")),
    );
    if standalone {
        page("Agent Reliability Card", &article)
    } else {
        article
    }
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><link rel=\"stylesheet\" href=\"style.css\"></head><body><main><div class=\"masthead\"><p class=\"eyebrow\">Open, local-first evidence</p><h1>{}</h1><p>False-green risk and verified completion are reported separately. Statistical output never overrides a categorical red gate.</p></div>{}</main></body></html>\n",
        html_escape(title),
        html_escape(title),
        body
    )
}

const CSS: &str = r#":root{color-scheme:light;--ink:#17211b;--paper:#f4f1e8;--card:#fffdf7;--accent:#135f46;--line:#d6d0bf;--warn:#9a3412}*{box-sizing:border-box}body{margin:0;background:var(--paper);color:var(--ink);font:16px/1.5 system-ui,-apple-system,Segoe UI,sans-serif}main{width:min(1180px,calc(100% - 32px));margin:0 auto;padding:56px 0}.masthead{max-width:760px;margin-bottom:36px}.masthead h1{font:700 clamp(2.5rem,7vw,5.5rem)/.95 Georgia,serif;margin:.15em 0}.eyebrow{text-transform:uppercase;letter-spacing:.12em;font-size:.75rem;font-weight:800;color:var(--accent)}.card{background:var(--card);border:1px solid var(--line);border-radius:18px;padding:clamp(20px,4vw,44px);margin:24px 0;box-shadow:0 15px 45px #17211b12}.card header{position:relative}.card h2{font:700 2rem/1.1 Georgia,serif;margin:.25rem 0}.status{display:inline-block;background:var(--ink);color:white;border-radius:999px;padding:5px 11px;font-size:.8rem}.headline{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px;margin:28px 0}.headline section{border-top:4px solid var(--accent);padding:18px;background:#edf5f0}.headline span,.headline small{display:block}.headline strong{display:block;font:700 clamp(2.1rem,6vw,4rem)/1 Georgia,serif;margin:.2em 0}.headline small,.quiet{color:#526057}dl{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:1px;background:var(--line);border:1px solid var(--line)}dl div{background:var(--card);padding:14px}dt{font-size:.76rem;text-transform:uppercase;letter-spacing:.07em;color:#667068}dd{font-weight:750;margin:4px 0 0;overflow-wrap:anywhere}.warnings{color:var(--warn)}.versions{border-top:1px solid var(--line);margin-top:20px}.versions p{overflow-wrap:anywhere}@media(max-width:760px){main{padding:28px 0}.headline,dl{grid-template-columns:1fr 1fr}}@media(max-width:480px){.headline,dl{grid-template-columns:1fr}}"#;

pub fn build_cards(data_dir: &Path, out_dir: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    if out_dir.exists() && fs::read_dir(out_dir)?.next().is_some() {
        return Err(
            "cards output directory must be empty so stale files cannot survive a build".into(),
        );
    }
    let cards = load_verified_public_cards(data_dir)?;
    fs::create_dir_all(out_dir)?;
    let static_cards = StaticCards {
        format_version: "1",
        card_generator_version: CARD_GENERATOR_VERSION,
        data_license: "CC-BY-4.0",
        cards: &cards,
    };
    let mut json = serde_json::to_vec_pretty(&static_cards)?;
    json.push(b'\n');
    scan_public_text(&String::from_utf8(json.clone())?)?;
    fs::write(out_dir.join("cards.json"), json)?;
    fs::write(out_dir.join("style.css"), CSS.as_bytes())?;
    let articles = cards
        .iter()
        .map(|card| card_html(card, false))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        out_dir.join("index.html"),
        page("Agent Reliability Cards", &articles),
    )?;
    for card in &cards {
        fs::write(
            out_dir.join(format!("{}.html", card.public_id)),
            card_html(card, true),
        )?;
    }
    Ok(cards.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_segments_reject_traversal() {
        assert!(safe_segment("release-1.2"));
        assert!(!safe_segment("../release"));
        assert!(!safe_segment("release/path"));
    }

    #[test]
    fn secret_scanner_has_a_deliberate_red_fixture() {
        assert!(scan_public_text("github_pat_deliberately-broken").is_err());
        assert!(scan_public_text("ordinary public reliability data").is_ok());
    }
}
