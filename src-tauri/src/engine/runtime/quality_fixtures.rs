//! Deterministic synthetic fixture loading for H2 shadow-quality evaluation.

use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::engine::runtime::quality::QualityTag;

const MANIFEST_JSON: &str = include_str!("quality_fixtures/h2-adversarial-v1.json");
const MANIFEST_ID: &str = "h2-adversarial-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedBehaviorClass {
    Accepted,
    Rejected,
    NearThreshold,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FixtureExpected {
    pub critical_facts: Vec<String>,
    pub expected_behavior_class: ExpectedBehaviorClass,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureCase {
    pub id: String,
    pub family: String,
    pub tags: Vec<QualityTag>,
    pub original_messages: Value,
    pub aggregate_summary: String,
    pub tail_start: usize,
    pub expected: FixtureExpected,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LoadedFixture {
    pub case: FixtureCase,
    pub plan: ctxopt::summary::SummaryPlan,
    pub projection: ctxopt::summary::SummaryProjection,
    pub source_envelope: String,
    pub original_hash: String,
    pub projected_hash: String,
    pub summary_hash: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FixtureError {
    #[error("fixture manifest JSON is invalid")]
    InvalidJson,
    #[error("fixture manifest id or variant count is invalid")]
    InvalidManifest,
    #[error("fixture id is duplicated: {0}")]
    DuplicateId(String),
    #[error("fixture tag is unknown or duplicated: {0}")]
    InvalidTag(String),
    #[error("fixture planning failed: {0}")]
    Planning(String),
    #[error("fixture projection was rejected: {0}")]
    Projection(String),
    #[error("fixture tag minimum was not met: {0}")]
    TagMinimum(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestFile {
    manifest: String,
    variants_per_template: usize,
    templates: Vec<FixtureTemplate>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureTemplate {
    id: String,
    family: String,
    tags: Vec<String>,
    original_messages: Value,
    aggregate_summary: String,
    tail_start: usize,
    expected: FixtureExpected,
}

fn replace_placeholders(value: &mut Value, variant: usize) {
    match value {
        Value::String(text) => {
            let padding = format!("evidence-{variant} ").repeat(80);
            *text = text
                .replace("{{VARIANT}}", &format!("{variant:02}"))
                .replace("{{PAD}}", &padding);
        }
        Value::Array(values) => {
            for value in values {
                replace_placeholders(value, variant);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace_placeholders(value, variant);
            }
        }
        _ => {}
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn parse_tags(labels: &[String]) -> Result<Vec<QualityTag>, FixtureError> {
    let mut tags = Vec::with_capacity(labels.len());
    let mut seen = HashSet::new();
    for label in labels {
        let tag =
            QualityTag::parse(label).ok_or_else(|| FixtureError::InvalidTag(label.clone()))?;
        if !seen.insert(tag) {
            return Err(FixtureError::InvalidTag(label.clone()));
        }
        tags.push(tag);
    }
    Ok(tags)
}

pub fn load_h2_adversarial_manifest() -> Result<Vec<LoadedFixture>, FixtureError> {
    let manifest: ManifestFile =
        serde_json::from_str(MANIFEST_JSON).map_err(|_| FixtureError::InvalidJson)?;
    if manifest.manifest != MANIFEST_ID
        || manifest.variants_per_template == 0
        || manifest.templates.is_empty()
    {
        return Err(FixtureError::InvalidManifest);
    }

    let mut loaded = Vec::new();
    let mut ids = HashSet::new();
    let mut tag_counts: HashMap<QualityTag, usize> = HashMap::new();
    for template in manifest.templates {
        let tags = parse_tags(&template.tags)?;
        for variant in 1..=manifest.variants_per_template {
            let id = format!("{}-{variant:02}", template.id);
            if !ids.insert(id.clone()) {
                return Err(FixtureError::DuplicateId(id));
            }
            let mut original_messages = template.original_messages.clone();
            replace_placeholders(&mut original_messages, variant);
            let aggregate_summary = template
                .aggregate_summary
                .replace("{{VARIANT}}", &format!("{variant:02}"));
            let plan = ctxopt::summary::plan_summary_span(&original_messages, template.tail_start)
                .map_err(|error| FixtureError::Planning(format!("{id}: {error:?}")))?;
            let projection = ctxopt::summary::build_summary_projection(
                &original_messages,
                &plan,
                &aggregate_summary,
            );
            if !matches!(projection.outcome, ctxopt::summary::SummaryOutcome::Applied) {
                return Err(FixtureError::Projection(format!(
                    "{id}: {:?}",
                    projection.outcome
                )));
            }
            let source_envelope = ctxopt::summary::render_untrusted_sources(&plan);
            serde_json::from_str::<Value>(&source_envelope)
                .map_err(|_| FixtureError::Projection(format!("{id}: invalid source envelope")))?;
            for tag in &tags {
                *tag_counts.entry(*tag).or_insert(0) += 1;
            }
            loaded.push(LoadedFixture {
                original_hash: sha256_hex(&serde_json::to_vec(&original_messages).unwrap()),
                projected_hash: sha256_hex(
                    &serde_json::to_vec(&projection.projected_messages).unwrap(),
                ),
                summary_hash: sha256_hex(aggregate_summary.as_bytes()),
                source_envelope,
                plan,
                projection,
                case: FixtureCase {
                    id,
                    family: template.family.clone(),
                    tags: tags.clone(),
                    original_messages,
                    aggregate_summary,
                    tail_start: template.tail_start,
                    expected: template.expected.clone(),
                },
            });
        }
    }

    for tag in QualityTag::ALL {
        if tag_counts.get(&tag).copied().unwrap_or(0) < 10 {
            return Err(FixtureError::TagMinimum(tag.as_str().to_owned()));
        }
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::runtime::quality::QualityTag;
    use ctxopt::summary::SummaryOutcome;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn manifest_expands_to_70_unique_h0_validated_cases_with_tag_minima() {
        let fixtures = load_h2_adversarial_manifest().unwrap();
        assert_eq!(fixtures.len(), 70);
        let ids: HashSet<_> = fixtures
            .iter()
            .map(|fixture| fixture.case.id.as_str())
            .collect();
        assert_eq!(ids.len(), fixtures.len());

        let mut tag_counts = HashMap::new();
        for fixture in &fixtures {
            for tag in &fixture.case.tags {
                *tag_counts.entry(*tag).or_insert(0usize) += 1;
            }
            assert_eq!(fixture.projection.outcome, SummaryOutcome::Applied);
            assert!(!fixture.plan.sources.is_empty());
        }
        for tag in QualityTag::ALL {
            assert!(tag_counts.get(&tag).copied().unwrap_or(0) >= 10);
        }
        let behavior_classes: HashSet<_> = fixtures
            .iter()
            .map(|fixture| fixture.case.expected.expected_behavior_class)
            .collect();
        assert_eq!(
            behavior_classes,
            HashSet::from([
                ExpectedBehaviorClass::Accepted,
                ExpectedBehaviorClass::Rejected,
                ExpectedBehaviorClass::NearThreshold,
            ])
        );
        assert_eq!(fixtures, load_h2_adversarial_manifest().unwrap());
    }

    #[test]
    fn prompt_injection_fixture_remains_a_json_string_value() {
        let fixtures = load_h2_adversarial_manifest().unwrap();
        let fixture = fixtures
            .iter()
            .find(|fixture| fixture.case.tags.contains(&QualityTag::PromptLikeToolText))
            .unwrap();
        let envelope: serde_json::Value = serde_json::from_str(&fixture.source_envelope).unwrap();
        let text = envelope["untrusted_tool_results"][0]["result_text"]
            .as_str()
            .unwrap();
        assert!(text.contains("</untrusted_tool_result>"));
        assert_eq!(
            envelope["untrusted_tool_results"].as_array().unwrap().len(),
            1
        );
    }
}
