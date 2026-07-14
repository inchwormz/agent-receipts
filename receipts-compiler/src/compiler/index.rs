//! Versioned Agentic Coding Reliability Index release gate.
//!
//! The builder emits ineligibility reasons for incomplete variants and never
//! assigns an index until every fixed public-data and calibration gate passes.

use crate::compiler::crypto::{ExecutorIdentity, hex_decode, sign_detached, verify_detached};
use crate::compiler::publication::{PublicReliabilityCard, load_verified_public_cards};
use crate::compiler::report::html_escape;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const INDEX_HASH_DOMAIN: &[u8] = b"agent-receipts:reliability-index:v1:hash";
const INDEX_SIGNATURE_DOMAIN: &[u8] = b"agent-receipts:reliability-index:v1:signature";
const INDEX_METHODOLOGY: &str = "fixed-equal-task-family-upper-bound-v1";
const CARD_GENERATOR_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskFamilyPin {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskMix {
    format_version: String,
    release_id: String,
    families: Vec<TaskFamilyPin>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct IndexVariant {
    pub variant_id: String,
    pub eligible: bool,
    pub ineligibility_reasons: Vec<String>,
    pub reliability_index: Option<f64>,
    pub equal_weight_upper_95_false_green_risk: Option<f64>,
    pub verified_completion_lower_95: Option<f64>,
    pub raw_sample_size: u64,
    pub effective_sample_size: f64,
    pub task_coverage: Vec<TaskFamilyPin>,
    pub mean_cost_usd: Option<f64>,
    pub calibration_status: String,
    pub provider: Option<String>,
    pub resolved_model_snapshot: Option<String>,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub harness_versions: Vec<String>,
    pub judge_identities: Vec<String>,
    pub dataset_hashes: Vec<String>,
    pub calibration_bundle_hashes: Vec<String>,
    pub methodology_versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReliabilityIndexRelease {
    pub format_version: String,
    pub record_kind: String,
    pub release_id: String,
    pub release_status: String,
    pub index_methodology: String,
    pub card_generator_version: String,
    pub task_mix: Vec<TaskFamilyPin>,
    pub task_mix_hash_alg: String,
    pub task_mix_hash: String,
    pub input_card_hashes: Vec<String>,
    pub variants: Vec<IndexVariant>,
    pub executor: ExecutorIdentity,
    pub hash_alg: String,
    pub signature_alg: String,
    pub record_hash: String,
    pub signature: String,
}

#[derive(Debug)]
struct GateInputs {
    effective_outcomes: f64,
    family_effective: Vec<(String, f64)>,
    missing_families: Vec<String>,
    calibrated: bool,
    maximum_interval_width: Option<f64>,
    exact_identity: bool,
    harness_identity: bool,
    judge_identity: bool,
    conflicting_family_cards: Vec<String>,
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

fn fixed_gate_reasons(inputs: &GateInputs) -> Vec<String> {
    let mut reasons = Vec::new();
    if inputs.effective_outcomes < 200.0 {
        reasons.push(format!(
            "requires at least 200 effective public outcomes (found {:.2})",
            inputs.effective_outcomes
        ));
    }
    if !inputs.missing_families.is_empty() {
        reasons.push(format!(
            "missing required task families: {}",
            inputs.missing_families.join(", ")
        ));
    }
    let under_sampled: Vec<String> = inputs
        .family_effective
        .iter()
        .filter(|(_, count)| *count < 30.0)
        .map(|(family, count)| format!("{family}={count:.2}"))
        .collect();
    if inputs.family_effective.len() < 3 || !under_sampled.is_empty() {
        reasons.push(format!(
            "requires three versioned task families with 30 effective outcomes each{}",
            if under_sampled.is_empty() {
                String::new()
            } else {
                format!(" (under threshold: {})", under_sampled.join(", "))
            }
        ));
    }
    if !inputs.calibrated {
        reasons.push(
            "all task-family cards must pass held-out calibration and be calibrated".to_string(),
        );
    }
    if inputs
        .maximum_interval_width
        .is_none_or(|width| width > 0.10)
    {
        reasons.push(
            "false-green 95% interval must be no wider than 0.10 for every family".to_string(),
        );
    }
    if !inputs.exact_identity {
        reasons.push(
            "exact provider, resolved model, agent name, and agent version are required"
                .to_string(),
        );
    }
    if !inputs.harness_identity {
        reasons.push("exact versioned harness identity is required".to_string());
    }
    if !inputs.judge_identity {
        reasons.push("exact authenticated judge identity is required".to_string());
    }
    if !inputs.conflicting_family_cards.is_empty() {
        reasons.push(format!(
            "conflicting calibration cards exist for task families: {}",
            inputs.conflicting_family_cards.join(", ")
        ));
    }
    reasons
}

fn variant_id(card: &PublicReliabilityCard) -> String {
    let versions = &card.metrics.versions;
    format!(
        "{}:{}:{}:{}",
        versions.provider.as_deref().unwrap_or("unresolved"),
        versions
            .resolved_model_snapshot
            .as_deref()
            .unwrap_or("unresolved"),
        versions.agent_name.as_deref().unwrap_or("unresolved"),
        versions.agent_version.as_deref().unwrap_or("unresolved")
    )
}

fn distinct_values<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn build_variant(
    id: String,
    cards: &[&PublicReliabilityCard],
    mix: &[TaskFamilyPin],
) -> IndexVariant {
    let first = cards[0];
    let mut selected = Vec::new();
    let mut missing = Vec::new();
    let mut conflicts = Vec::new();
    for family in mix {
        let matching: Vec<_> = cards
            .iter()
            .copied()
            .filter(|card| card.metrics.task_family == family.id)
            .collect();
        if matching.is_empty() {
            missing.push(format!("{}@{}", family.id, family.version));
            continue;
        }
        let signatures: BTreeSet<String> = matching
            .iter()
            .map(|card| {
                format!(
                    "{}:{}:{}:{:.12}:{}",
                    card.metrics.versions.dataset_hash,
                    card.metrics.versions.calibration_bundle_hash,
                    card.metrics.versions.methodology_version,
                    card.metrics.effective_sample_size,
                    card.metrics
                        .upper_95_false_green_risk
                        .map_or_else(|| "none".to_string(), |value| format!("{value:.12}"))
                )
            })
            .collect();
        if signatures.len() > 1 {
            conflicts.push(format!("{}@{}", family.id, family.version));
        }
        selected.push((family.clone(), matching[0]));
    }

    let effective = selected
        .iter()
        .map(|(_, card)| card.metrics.effective_sample_size)
        .sum::<f64>();
    let raw = selected
        .iter()
        .map(|(_, card)| card.metrics.raw_sample_size)
        .sum();
    let family_effective = selected
        .iter()
        .map(|(family, card)| {
            (
                format!("{}@{}", family.id, family.version),
                card.metrics.effective_sample_size,
            )
        })
        .collect();
    let exact_identity = selected.iter().all(|(_, card)| {
        let versions = &card.metrics.versions;
        versions
            .provider
            .as_ref()
            .is_some_and(|value| !value.is_empty())
            && versions
                .resolved_model_snapshot
                .as_ref()
                .is_some_and(|value| !value.is_empty())
            && versions
                .agent_name
                .as_ref()
                .is_some_and(|value| !value.is_empty())
            && versions
                .agent_version
                .as_ref()
                .is_some_and(|value| !value.is_empty())
    });
    let calibrated = selected.iter().all(|(_, card)| {
        card.metrics.calibration_status == "calibrated"
            && card.metrics.score_status == "calibrated"
            && card.metrics.held_out_calibration_passed
    });
    let maximum_interval_width = selected
        .iter()
        .filter_map(|(_, card)| card.metrics.false_green_interval_95_width)
        .max_by(f64::total_cmp)
        .filter(|_| {
            selected
                .iter()
                .all(|(_, card)| card.metrics.false_green_interval_95_width.is_some())
        });
    let harness_identity = selected
        .iter()
        .all(|(_, card)| !card.metrics.versions.harness_versions.is_empty());
    let judge_identity = selected
        .iter()
        .all(|(_, card)| !card.metrics.versions.judge_identities.is_empty());
    let reasons = fixed_gate_reasons(&GateInputs {
        effective_outcomes: effective,
        family_effective,
        missing_families: missing,
        calibrated,
        maximum_interval_width,
        exact_identity,
        harness_identity,
        judge_identity,
        conflicting_family_cards: conflicts,
    });
    let eligible = reasons.is_empty();
    let upper = eligible.then(|| {
        selected
            .iter()
            .map(|(_, card)| card.metrics.upper_95_false_green_risk.unwrap())
            .sum::<f64>()
            / selected.len() as f64
    });
    let completion_lower = eligible.then(|| {
        selected
            .iter()
            .map(|(_, card)| card.metrics.verified_completion.as_ref().unwrap().lower_95)
            .sum::<f64>()
            / selected.len() as f64
    });
    let costs: Vec<f64> = selected
        .iter()
        .filter_map(|(_, card)| card.metrics.cost_usd)
        .collect();
    IndexVariant {
        variant_id: id,
        eligible,
        ineligibility_reasons: reasons,
        reliability_index: upper.map(|risk| 100.0 * (1.0 - risk)),
        equal_weight_upper_95_false_green_risk: upper,
        verified_completion_lower_95: completion_lower,
        raw_sample_size: raw,
        effective_sample_size: effective,
        task_coverage: selected.into_iter().map(|(family, _)| family).collect(),
        mean_cost_usd: (!costs.is_empty()).then(|| costs.iter().sum::<f64>() / costs.len() as f64),
        calibration_status: if calibrated {
            "calibrated".to_string()
        } else {
            "ineligible".to_string()
        },
        provider: first.metrics.versions.provider.clone(),
        resolved_model_snapshot: first.metrics.versions.resolved_model_snapshot.clone(),
        agent_name: first.metrics.versions.agent_name.clone(),
        agent_version: first.metrics.versions.agent_version.clone(),
        harness_versions: distinct_values(cards.iter().flat_map(|card| {
            card.metrics
                .versions
                .harness_versions
                .iter()
                .map(String::as_str)
        })),
        judge_identities: distinct_values(cards.iter().flat_map(|card| {
            card.metrics
                .versions
                .judge_identities
                .iter()
                .map(String::as_str)
        })),
        dataset_hashes: distinct_values(
            cards
                .iter()
                .map(|card| card.metrics.versions.dataset_hash.as_str()),
        ),
        calibration_bundle_hashes: distinct_values(
            cards
                .iter()
                .map(|card| card.metrics.versions.calibration_bundle_hash.as_str()),
        ),
        methodology_versions: distinct_values(
            cards
                .iter()
                .map(|card| card.metrics.versions.methodology_version.as_str()),
        ),
    }
}

fn canonical_unsigned(
    release: &ReliabilityIndexRelease,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut canonical = release.clone();
    canonical.record_hash.clear();
    canonical.signature.clear();
    Ok(serde_json::to_vec(&canonical)?)
}

fn record_hash(release: &ReliabilityIndexRelease) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(INDEX_HASH_DOMAIN);
    hasher.update(&canonical_unsigned(release)?);
    Ok(hasher.finalize().to_hex().to_string())
}

fn verify_release(release: &ReliabilityIndexRelease) -> Result<(), Box<dyn std::error::Error>> {
    if release.format_version != "1"
        || release.record_kind != "agentic_coding_reliability_index"
        || !safe_segment(&release.release_id)
        || release.index_methodology != INDEX_METHODOLOGY
        || release.card_generator_version != CARD_GENERATOR_VERSION
        || release.task_mix_hash_alg != "blake3-256"
        || release.hash_alg != "blake3-256"
        || release.signature_alg != "ed25519"
        || release.record_hash.len() != 64
        || release.signature.len() != 128
        || (release.release_status == "withheld"
            && release
                .variants
                .iter()
                .any(|variant| variant.reliability_index.is_some()))
    {
        return Err("reliability index release has unsupported metadata".into());
    }
    let actual = record_hash(release)?;
    if actual != release.record_hash {
        return Err("reliability index release hash mismatch".into());
    }
    verify_detached(
        INDEX_SIGNATURE_DOMAIN,
        &hex_decode(&release.record_hash)?,
        &release.executor,
        &release.signature,
    )
}

fn index_html(release: &ReliabilityIndexRelease) -> String {
    let variants = release
        .variants
        .iter()
        .map(|variant| {
            let number = variant.reliability_index.map_or_else(
                || "Withheld".to_string(),
                |value| format!("{value:.1}"),
            );
            let details = if variant.eligible {
                format!(
                    "Upper risk {:.1}% · verified completion lower bound {:.1}% · {:.2} effective outcomes",
                    variant.equal_weight_upper_95_false_green_risk.unwrap() * 100.0,
                    variant.verified_completion_lower_95.unwrap() * 100.0,
                    variant.effective_sample_size
                )
            } else {
                variant.ineligibility_reasons.join(" · ")
            };
            format!(
                "<article><p>{}</p><strong>{}</strong><h2>{}</h2><p>{}</p></article>",
                html_escape(&variant.calibration_status),
                html_escape(&number),
                html_escape(&variant.variant_id),
                html_escape(&details)
            )
        })
        .collect::<String>();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Agentic Coding Reliability Index</title><style>body{{margin:0;background:#101713;color:#eef4ef;font:16px/1.5 system-ui}}main{{width:min(1050px,calc(100% - 32px));margin:auto;padding:64px 0}}h1{{font:700 clamp(2.7rem,8vw,6rem)/.95 Georgia,serif;max-width:900px}}.meta{{color:#a8b8ad}}article{{background:#17231b;border:1px solid #365140;border-radius:16px;padding:28px;margin:18px 0}}article strong{{float:right;font:700 3rem/1 Georgia,serif;color:#74d69a}}article h2{{overflow-wrap:anywhere}}@media(max-width:600px){{article strong{{float:none;display:block}}}}</style></head><body><main><p class=\"meta\">{} · {}</p><h1>Agentic Coding Reliability Index</h1><p>100 × (1 − upper 95% posterior false-green risk), across a fixed equal-weight task-family mix. Missing evidence is never imputed.</p>{}</main></body></html>\n",
        html_escape(&release.release_id),
        html_escape(&release.release_status),
        variants
    )
}

pub fn build_index(
    data_dir: &Path,
    task_mix_path: &Path,
    out_dir: &Path,
) -> Result<ReliabilityIndexRelease, Box<dyn std::error::Error>> {
    if out_dir.exists() && fs::read_dir(out_dir)?.next().is_some() {
        return Err("index output directory must be empty; releases are immutable".into());
    }
    let task_mix_bytes = fs::read(task_mix_path)?;
    let mix: TaskMix = serde_json::from_slice(&task_mix_bytes)?;
    if mix.format_version != "1" || !safe_segment(&mix.release_id) || mix.families.len() < 3 {
        return Err(
            "task mix must pin a safe release id and at least three versioned families".into(),
        );
    }
    let mut family_ids = BTreeSet::new();
    for family in &mix.families {
        if !safe_segment(&family.id)
            || !safe_segment(&family.version)
            || !family_ids.insert(family.id.clone())
        {
            return Err("task mix family ids and versions must be unique safe segments".into());
        }
    }
    let cards = load_verified_public_cards(data_dir)?;
    let mut grouped: BTreeMap<String, Vec<&PublicReliabilityCard>> = BTreeMap::new();
    for card in &cards {
        grouped.entry(variant_id(card)).or_default().push(card);
    }
    let variants: Vec<IndexVariant> = grouped
        .into_iter()
        .map(|(id, cards)| build_variant(id, &cards, &mix.families))
        .collect();
    let release_status = if variants.iter().any(|variant| variant.eligible) {
        "published"
    } else {
        "withheld"
    };
    let mut release = ReliabilityIndexRelease {
        format_version: "1".to_string(),
        record_kind: "agentic_coding_reliability_index".to_string(),
        release_id: mix.release_id,
        release_status: release_status.to_string(),
        index_methodology: INDEX_METHODOLOGY.to_string(),
        card_generator_version: CARD_GENERATOR_VERSION.to_string(),
        task_mix: mix.families,
        task_mix_hash_alg: "blake3-256".to_string(),
        task_mix_hash: blake3::hash(&task_mix_bytes).to_hex().to_string(),
        input_card_hashes: cards.iter().map(|card| card.record_hash.clone()).collect(),
        variants,
        executor: crate::compiler::crypto::current_executor_identity()?,
        hash_alg: "blake3-256".to_string(),
        signature_alg: "ed25519".to_string(),
        record_hash: String::new(),
        signature: String::new(),
    };
    release.input_card_hashes.sort();
    release.record_hash = record_hash(&release)?;
    let (executor, signature) =
        sign_detached(INDEX_SIGNATURE_DOMAIN, &hex_decode(&release.record_hash)?)?;
    if executor != release.executor {
        return Err("executor identity changed while signing reliability index".into());
    }
    release.signature = signature;
    verify_release(&release)?;
    fs::create_dir_all(out_dir)?;
    let mut bytes = serde_json::to_vec_pretty(&release)?;
    bytes.push(b'\n');
    fs::write(out_dir.join("reliability-index.json"), bytes)?;
    fs::write(out_dir.join("index.html"), index_html(&release))?;
    Ok(release)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligible_inputs() -> GateInputs {
        GateInputs {
            effective_outcomes: 200.0,
            family_effective: vec![
                ("bugfix@v1".into(), 70.0),
                ("refactor@v1".into(), 65.0),
                ("feature@v1".into(), 65.0),
            ],
            missing_families: vec![],
            calibrated: true,
            maximum_interval_width: Some(0.10),
            exact_identity: true,
            harness_identity: true,
            judge_identity: true,
            conflicting_family_cards: vec![],
        }
    }

    #[test]
    fn fixed_index_gates_have_exact_boundaries() {
        assert!(fixed_gate_reasons(&eligible_inputs()).is_empty());
        let mut under = eligible_inputs();
        under.effective_outcomes = 199.99;
        assert!(fixed_gate_reasons(&under)[0].contains("200"));
        let mut wide = eligible_inputs();
        wide.maximum_interval_width = Some(0.10001);
        assert!(
            fixed_gate_reasons(&wide)
                .iter()
                .any(|reason| reason.contains("0.10"))
        );
        let mut missing = eligible_inputs();
        missing.missing_families.push("feature@v1".into());
        assert!(
            fixed_gate_reasons(&missing)
                .iter()
                .any(|reason| reason.contains("missing"))
        );
    }
}
