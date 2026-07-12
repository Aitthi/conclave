//! Pure contracts for H2 shadow-quality evaluation.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const QUALITY_METHOD_VERSION: &str = "h2-shadow-quality-v1";
pub const QUALITY_RUBRIC_VERSION: &str = "hybrid-quality-rubric-v1";
pub const PROBE_PROMPT_VERSION: &str = "source-probes-v1";
pub const FAITHFULNESS_PROMPT_VERSION: &str = "faithfulness-v1";
pub const REPLAY_PROMPT_VERSION: &str = "next-action-v1";
pub const JUDGE_PROMPT_VERSION: &str = "blind-next-action-v1";
pub const BOOTSTRAP_METHOD_VERSION: &str = "paired-bootstrap-v1";
pub const MAX_PROBES: usize = 32;
pub const MAX_CALLS_PER_CASE: u64 = 5;
pub const QUALITY_CARRIER_COOLDOWN_MS: u64 = 60_000;
pub const MAX_FIXTURES_PER_CARRIER: usize = 3;

pub const CRITICALITY_RUBRIC: &str = "A fact is critical when losing or fabricating it can change the user goal, a constraint or acceptance criterion, whether a mutation or side effect occurred, the next safe action, an unresolved blocker, the conclusion of negative evidence, or an exact identifier, path, command, version, error, or number required to continue or diagnose. Style, ordering, and non-load-bearing wording are noncritical.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualityTag {
    SideEffectingOutput,
    LongLog,
    ExactError,
    RejectedAlternative,
    ParallelToolCycle,
    PromptLikeToolText,
    MutationOrOpenWork,
}

impl QualityTag {
    pub const ALL: [Self; 7] = [
        Self::SideEffectingOutput,
        Self::LongLog,
        Self::ExactError,
        Self::RejectedAlternative,
        Self::ParallelToolCycle,
        Self::PromptLikeToolText,
        Self::MutationOrOpenWork,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SideEffectingOutput => "side_effecting_output",
            Self::LongLog => "long_log",
            Self::ExactError => "exact_error",
            Self::RejectedAlternative => "rejected_alternative",
            Self::ParallelToolCycle => "parallel_tool_cycle",
            Self::PromptLikeToolText => "prompt_like_tool_text",
            Self::MutationOrOpenWork => "mutation_or_open_work",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tag| tag.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProbeCategory {
    Constraint,
    Decision,
    Mutation,
    ExactIdentifierOrError,
    NegativeFinding,
    OpenWork,
}

impl ProbeCategory {
    pub const ALL: [Self; 6] = [
        Self::Constraint,
        Self::Decision,
        Self::Mutation,
        Self::ExactIdentifierOrError,
        Self::NegativeFinding,
        Self::OpenWork,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Constraint => "constraint",
            Self::Decision => "decision",
            Self::Mutation => "mutation",
            Self::ExactIdentifierOrError => "exact_identifier_or_error",
            Self::NegativeFinding => "negative_finding",
            Self::OpenWork => "open_work",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Criticality {
    Critical,
    Noncritical,
}

impl Criticality {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Noncritical => "noncritical",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualitySource {
    Live,
    Fixture {
        id: String,
        family: String,
        tags: Vec<QualityTag>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityRole {
    Probe,
    Faithfulness,
    OriginalReplay,
    ProjectedReplay,
    Judge,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualityCall {
    pub role: QualityRole,
    pub request: Value,
}

impl QualityCall {
    pub fn new(role: QualityRole, request: Value) -> Self {
        Self { role, request }
    }
}

pub struct QualityCaseCalls {
    pub probe: QualityCall,
    pub faithfulness: QualityCall,
    /// The caller randomizes which replay occupies this slot.
    pub replay_first: QualityCall,
    pub replay_second: QualityCall,
    pub judge: QualityCall,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualityCaseResponses {
    pub probe: Value,
    pub faithfulness: Value,
    pub replay_first: Value,
    pub replay_second: Value,
    pub judge: Value,
}

pub type QualityTransportFuture<'a, E> =
    Pin<Box<dyn Future<Output = Result<Value, E>> + Send + 'a>>;

/// One content-agnostic transport seam. Lane B implements the real no-redirect
/// provider transport; tests use [`MockQualityTransport`].
pub trait QualityTransport: Sync {
    type Error;

    fn send(&self, call: QualityCall) -> QualityTransportFuture<'_, Self::Error>;
}

/// Execute exactly the five quality roles in the order supplied by the caller.
/// Replay order is intentionally external so the runtime can randomize it and
/// retain the opaque mapping for the blind judge.
pub async fn evaluate_quality_case<T: QualityTransport>(
    transport: &T,
    calls: QualityCaseCalls,
) -> Result<QualityCaseResponses, T::Error> {
    let probe = transport.send(calls.probe).await?;
    let faithfulness = transport.send(calls.faithfulness).await?;
    let replay_first = transport.send(calls.replay_first).await?;
    let replay_second = transport.send(calls.replay_second).await?;
    let judge = transport.send(calls.judge).await?;
    Ok(QualityCaseResponses {
        probe,
        faithfulness,
        replay_first,
        replay_second,
        judge,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockQualityError {
    Exhausted,
    Scripted(&'static str),
}

impl std::fmt::Display for MockQualityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Exhausted => write!(f, "mock quality responses exhausted"),
            Self::Scripted(kind) => write!(f, "mock quality failure: {kind}"),
        }
    }
}

impl std::error::Error for MockQualityError {}

pub struct MockQualityTransport {
    responses: Mutex<VecDeque<Result<Value, MockQualityError>>>,
    calls: Mutex<Vec<QualityCall>>,
}

impl MockQualityTransport {
    pub fn scripted(responses: impl IntoIterator<Item = Value>) -> Self {
        Self::scripted_results(responses.into_iter().map(Ok))
    }

    pub fn scripted_results(
        responses: impl IntoIterator<Item = Result<Value, MockQualityError>>,
    ) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<QualityCall> {
        self.calls.lock().unwrap().clone()
    }

    pub fn roles(&self) -> Vec<QualityRole> {
        self.calls().into_iter().map(|call| call.role).collect()
    }
}

impl QualityTransport for MockQualityTransport {
    type Error = MockQualityError;

    fn send(&self, call: QualityCall) -> QualityTransportFuture<'_, Self::Error> {
        self.calls.lock().unwrap().push(call);
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Err(MockQualityError::Exhausted));
        Box::pin(async move { response })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviorSource {
    Live,
    Fixture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BehaviorCase {
    pub source: BehaviorSource,
    pub cluster: String,
    pub original_pass: bool,
    pub projected_pass: bool,
}

impl BehaviorCase {
    pub fn live(cluster: impl Into<String>, original_pass: bool, projected_pass: bool) -> Self {
        Self {
            source: BehaviorSource::Live,
            cluster: cluster.into(),
            original_pass,
            projected_pass,
        }
    }

    pub fn fixture(cluster: impl Into<String>, original_pass: bool, projected_pass: bool) -> Self {
        Self {
            source: BehaviorSource::Fixture,
            cluster: cluster.into(),
            original_pass,
            projected_pass,
        }
    }

    fn difference(&self) -> i8 {
        i8::from(self.projected_pass) - i8::from(self.original_pass)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapResult {
    pub point_estimate: f64,
    pub ci_lower: f64,
    pub ci_upper: f64,
    pub replicates: usize,
    pub method_version: &'static str,
    pub seed_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapError {
    NoCases,
    InsufficientLiveClusters,
    InsufficientFixtureFamilies,
}

const BOOTSTRAP_REPLICATES: usize = 10_000;

fn clusters(cases: &[BehaviorCase], source: BehaviorSource) -> Vec<Vec<i8>> {
    let mut grouped: BTreeMap<&str, Vec<i8>> = BTreeMap::new();
    for case in cases.iter().filter(|case| case.source == source) {
        grouped
            .entry(case.cluster.as_str())
            .or_default()
            .push(case.difference());
    }
    grouped.into_values().collect()
}

fn sampled_cluster_mean(clusters: &[Vec<i8>], rng: &mut ChaCha20Rng) -> f64 {
    let mut total = 0i64;
    let mut count = 0usize;
    for _ in 0..clusters.len() {
        let selected = &clusters[(rng.next_u64() as usize) % clusters.len()];
        total += selected.iter().map(|value| i64::from(*value)).sum::<i64>();
        count += selected.len();
    }
    total as f64 / count as f64
}

fn sha256_hex(bytes: &[u8]) -> (String, [u8; 32]) {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    (hex, digest)
}

pub fn paired_cluster_bootstrap(
    quality_campaign_id: &str,
    rubric_version: &str,
    cases: &[BehaviorCase],
) -> Result<BootstrapResult, BootstrapError> {
    if cases.is_empty() {
        return Err(BootstrapError::NoCases);
    }
    let live = clusters(cases, BehaviorSource::Live);
    let fixtures = clusters(cases, BehaviorSource::Fixture);
    if live.len() < 2 {
        return Err(BootstrapError::InsufficientLiveClusters);
    }
    if fixtures.len() < 2 {
        return Err(BootstrapError::InsufficientFixtureFamilies);
    }

    let seed_material = format!("{quality_campaign_id}{rubric_version}{BOOTSTRAP_METHOD_VERSION}");
    let (seed_hash, seed) = sha256_hex(seed_material.as_bytes());
    let mut rng = ChaCha20Rng::from_seed(seed);
    let live_count = cases
        .iter()
        .filter(|case| case.source == BehaviorSource::Live)
        .count();
    let fixture_count = cases.len() - live_count;
    let mut replicates = Vec::with_capacity(BOOTSTRAP_REPLICATES);
    for _ in 0..BOOTSTRAP_REPLICATES {
        let live_mean = sampled_cluster_mean(&live, &mut rng);
        let fixture_mean = sampled_cluster_mean(&fixtures, &mut rng);
        replicates.push(
            (live_mean * live_count as f64 + fixture_mean * fixture_count as f64)
                / cases.len() as f64,
        );
    }
    replicates.sort_by(f64::total_cmp);
    let percentile = |p: f64| replicates[(p * (replicates.len() - 1) as f64).floor() as usize];
    let point_estimate = cases
        .iter()
        .map(|case| f64::from(case.difference()))
        .sum::<f64>()
        / cases.len() as f64;

    Ok(BootstrapResult {
        point_estimate,
        ci_lower: percentile(0.025),
        ci_upper: percentile(0.975),
        replicates: BOOTSTRAP_REPLICATES,
        method_version: BOOTSTRAP_METHOD_VERSION,
        seed_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quality_versions_and_limits_are_pinned() {
        assert_eq!(QUALITY_METHOD_VERSION, "h2-shadow-quality-v1");
        assert_eq!(QUALITY_RUBRIC_VERSION, "hybrid-quality-rubric-v1");
        assert_eq!(PROBE_PROMPT_VERSION, "source-probes-v1");
        assert_eq!(FAITHFULNESS_PROMPT_VERSION, "faithfulness-v1");
        assert_eq!(REPLAY_PROMPT_VERSION, "next-action-v1");
        assert_eq!(JUDGE_PROMPT_VERSION, "blind-next-action-v1");
        assert_eq!(BOOTSTRAP_METHOD_VERSION, "paired-bootstrap-v1");
        assert_eq!(MAX_PROBES, 32);
        assert_eq!(MAX_CALLS_PER_CASE, 5);
        assert_eq!(QUALITY_CARRIER_COOLDOWN_MS, 60_000);
        assert_eq!(MAX_FIXTURES_PER_CARRIER, 3);
    }

    #[test]
    fn quality_tags_are_pinned_allowlisted_and_round_trip() {
        let labels: Vec<_> = QualityTag::ALL.iter().map(|tag| tag.as_str()).collect();
        assert_eq!(
            labels,
            [
                "side_effecting_output",
                "long_log",
                "exact_error",
                "rejected_alternative",
                "parallel_tool_cycle",
                "prompt_like_tool_text",
                "mutation_or_open_work",
            ]
        );
        for tag in QualityTag::ALL {
            assert_eq!(QualityTag::parse(tag.as_str()), Some(tag));
        }
        assert_eq!(QualityTag::parse("unknown"), None);
    }

    #[test]
    fn probe_categories_and_criticality_are_bounded() {
        let labels: Vec<_> = ProbeCategory::ALL
            .iter()
            .map(|category| category.as_str())
            .collect();
        assert_eq!(
            labels,
            [
                "constraint",
                "decision",
                "mutation",
                "exact_identifier_or_error",
                "negative_finding",
                "open_work",
            ]
        );
        assert_eq!(Criticality::Critical.as_str(), "critical");
        assert_eq!(Criticality::Noncritical.as_str(), "noncritical");
    }

    #[tokio::test]
    async fn mock_transport_drives_five_call_shape_in_supplied_blind_order() {
        let calls = QualityCaseCalls {
            probe: QualityCall::new(QualityRole::Probe, json!({"step": 1})),
            faithfulness: QualityCall::new(QualityRole::Faithfulness, json!({"step": 2})),
            replay_first: QualityCall::new(QualityRole::ProjectedReplay, json!({"step": 3})),
            replay_second: QualityCall::new(QualityRole::OriginalReplay, json!({"step": 4})),
            judge: QualityCall::new(QualityRole::Judge, json!({"step": 5})),
        };
        let transport = MockQualityTransport::scripted([
            json!({"response": 1}),
            json!({"response": 2}),
            json!({"response": 3}),
            json!({"response": 4}),
            json!({"response": 5}),
        ]);

        let responses = evaluate_quality_case(&transport, calls).await.unwrap();

        assert_eq!(responses.probe, json!({"response": 1}));
        assert_eq!(responses.judge, json!({"response": 5}));
        assert_eq!(
            transport.roles(),
            [
                QualityRole::Probe,
                QualityRole::Faithfulness,
                QualityRole::ProjectedReplay,
                QualityRole::OriginalReplay,
                QualityRole::Judge,
            ]
        );
    }

    #[tokio::test]
    async fn mock_transport_stops_after_a_scripted_call_failure() {
        let calls = QualityCaseCalls {
            probe: QualityCall::new(QualityRole::Probe, json!({"step": 1})),
            faithfulness: QualityCall::new(QualityRole::Faithfulness, json!({"step": 2})),
            replay_first: QualityCall::new(QualityRole::OriginalReplay, json!({"step": 3})),
            replay_second: QualityCall::new(QualityRole::ProjectedReplay, json!({"step": 4})),
            judge: QualityCall::new(QualityRole::Judge, json!({"step": 5})),
        };
        let transport = MockQualityTransport::scripted_results([
            Ok(json!({"response": 1})),
            Err(MockQualityError::Scripted("schema")),
            Ok(json!({"response": 3})),
        ]);

        let result = evaluate_quality_case(&transport, calls).await;

        assert_eq!(result, Err(MockQualityError::Scripted("schema")));
        assert_eq!(
            transport.roles(),
            [QualityRole::Probe, QualityRole::Faithfulness]
        );
    }

    #[test]
    fn paired_bootstrap_is_seeded_stratified_and_pinned() {
        let cases = [
            BehaviorCase::live("live-a", true, false),
            BehaviorCase::live("live-b", false, true),
            BehaviorCase::fixture("family-a", true, true),
            BehaviorCase::fixture("family-b", false, false),
        ];

        let result = paired_cluster_bootstrap("campaign", "rubric", &cases).unwrap();
        assert_eq!(result.replicates, 10_000);
        assert_eq!(result.point_estimate, 0.0);
        assert_eq!(result.ci_lower, -0.5);
        assert_eq!(result.ci_upper, 0.5);
        assert_eq!(result.method_version, BOOTSTRAP_METHOD_VERSION);
        assert_eq!(result.seed_hash.len(), 64);
        assert_eq!(
            result,
            paired_cluster_bootstrap("campaign", "rubric", &cases).unwrap()
        );
    }

    #[test]
    fn paired_bootstrap_pins_direction_and_percentile_edges() {
        let cases = [
            BehaviorCase::live("live-small", true, false),
            BehaviorCase::live("live-large", false, true),
            BehaviorCase::live("live-large", false, true),
            BehaviorCase::live("live-large", false, true),
            BehaviorCase::fixture("fixture-small", true, false),
            BehaviorCase::fixture("fixture-large", true, true),
            BehaviorCase::fixture("fixture-large", true, true),
            BehaviorCase::fixture("fixture-large", false, true),
            BehaviorCase::fixture("fixture-large", false, true),
        ];

        let result = paired_cluster_bootstrap("asymmetric-campaign", "rubric", &cases).unwrap();
        assert_eq!(result.point_estimate, 1.0 / 3.0);
        assert_eq!(result.ci_lower, -1.0);
        assert_eq!(result.ci_upper, 13.0 / 18.0);
    }

    #[test]
    fn paired_bootstrap_requires_two_clusters_per_source() {
        let cases = [
            BehaviorCase::live("only-live", true, true),
            BehaviorCase::fixture("family-a", true, true),
            BehaviorCase::fixture("family-b", true, true),
        ];
        assert_eq!(
            paired_cluster_bootstrap("campaign", "rubric", &cases),
            Err(BootstrapError::InsufficientLiveClusters)
        );
    }
}
