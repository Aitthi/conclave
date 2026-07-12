//! Ephemeral, synthetic-only H2 human-audit reservoir and loopback page.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Form, Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::engine::repo::proxy_quality_metric::{AuditBucket, AuditVerdict};
use crate::engine::runtime::quality::{JudgeScores, QualityProbe, QualityTag};

const MAX_AUDIT_CASES: usize = 12;
const CASES_PER_BUCKET: usize = 4;
const AUDIT_TTL: Duration = Duration::from_secs(2 * 60 * 60);

/// Raw fixture material eligible for the audit reservoir. There is no live
/// variant: callers cannot offer a live case by construction.
pub struct FixtureAuditBundle {
    pub case_id: String,
    pub fixture_id: String,
    pub tags: Vec<QualityTag>,
    pub bucket: AuditBucket,
    source_json: String,
    summary: String,
    probes: String,
    original_plan: String,
    projected_plan: String,
    verifier: String,
    judge_a: String,
    judge_b: String,
    label_a_is_original: bool,
}

impl FixtureAuditBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn completed(
        case_id: String,
        fixture_id: &str,
        tags: Vec<QualityTag>,
        source_json: String,
        summary: String,
        probes: &[QualityProbe],
        original_plan: String,
        projected_plan: String,
        verifier: String,
        judge_a: JudgeScores,
        judge_b: JudgeScores,
        label_a_is_original: bool,
    ) -> Option<Self> {
        use crate::engine::runtime::quality_fixtures::ExpectedBehaviorClass;

        let fixture = crate::engine::runtime::quality_fixtures::load_h2_adversarial_manifest()
            .ok()?
            .into_iter()
            .find(|fixture| fixture.case.id == fixture_id)?;
        let bucket = match fixture.case.expected.expected_behavior_class {
            ExpectedBehaviorClass::Accepted => AuditBucket::Accepted,
            ExpectedBehaviorClass::Rejected => AuditBucket::Rejected,
            ExpectedBehaviorClass::NearThreshold => AuditBucket::NearThreshold,
        };
        let probes = serde_json::Value::Array(
            probes
                .iter()
                .map(|probe| {
                    serde_json::json!({
                        "id": probe.id,
                        "category": probe.category.as_str(),
                        "question": probe.question,
                        "expectedAnswer": probe.expected_answer,
                        "critical": probe.critical,
                    })
                })
                .collect(),
        )
        .to_string();
        Some(Self {
            case_id,
            fixture_id: fixture_id.to_owned(),
            tags,
            bucket,
            source_json,
            summary,
            probes,
            original_plan,
            projected_plan,
            verifier,
            judge_a: scores_json(judge_a),
            judge_b: scores_json(judge_b),
            label_a_is_original,
        })
    }

    fn wipe(&mut self) {
        wipe_string(&mut self.case_id);
        wipe_string(&mut self.fixture_id);
        wipe_string(&mut self.source_json);
        wipe_string(&mut self.summary);
        wipe_string(&mut self.probes);
        wipe_string(&mut self.original_plan);
        wipe_string(&mut self.projected_plan);
        wipe_string(&mut self.verifier);
        wipe_string(&mut self.judge_a);
        wipe_string(&mut self.judge_b);
        self.tags.clear();
    }

    #[cfg(test)]
    fn raw_bytes(&self) -> usize {
        self.source_json.len()
            + self.summary.len()
            + self.probes.len()
            + self.original_plan.len()
            + self.projected_plan.len()
            + self.verifier.len()
            + self.judge_a.len()
            + self.judge_b.len()
    }
}

impl Drop for FixtureAuditBundle {
    fn drop(&mut self) {
        self.wipe();
    }
}

fn scores_json(scores: JudgeScores) -> String {
    serde_json::json!({
        "correct": scores.correct,
        "constraintAdherent": scores.constraint_adherent,
        "nextActionMatch": scores.next_action_match,
    })
    .to_string()
}

fn wipe_string(value: &mut String) {
    // SAFETY: overwriting every byte with NUL preserves UTF-8 validity. The
    // length/capacity are unchanged until `clear`, so the raw bytes are erased
    // before the allocation can be released or reused.
    unsafe {
        for byte in value.as_mut_vec() {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    value.clear();
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditStatus {
    pub active: bool,
    pub selected: u64,
    pub submitted: u64,
    pub expires_in_seconds: u64,
}

pub struct AuditStart {
    pub url: String,
}

type AuditWriteFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

trait AuditWriter: Send + Sync {
    fn write(
        &self,
        case_id: &str,
        bucket: AuditBucket,
        verdict: AuditVerdict,
    ) -> AuditWriteFuture<'_>;
}

struct SqliteAuditWriter {
    pool: sqlx::SqlitePool,
    campaign_id: String,
}

impl AuditWriter for SqliteAuditWriter {
    fn write(
        &self,
        case_id: &str,
        bucket: AuditBucket,
        verdict: AuditVerdict,
    ) -> AuditWriteFuture<'_> {
        let case_id = case_id.to_owned();
        Box::pin(async move {
            let metric_id: Option<i64> = sqlx::query_scalar(
                "SELECT id FROM proxy_quality_metric WHERE case_id = ?1 AND quality_campaign_id = ?2",
            )
            .bind(case_id)
            .bind(&self.campaign_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
            let metric_id = metric_id.ok_or_else(|| "audit case is not persisted".to_owned())?;
            crate::engine::repo::proxy_quality_metric::insert_audit(
                &self.pool,
                metric_id,
                bucket,
                verdict,
                &chrono::Utc::now().to_rfc3339(),
            )
            .await
            .map_err(|error| error.to_string())
        })
    }
}

struct AuditEntry {
    bundle: FixtureAuditBundle,
    in_flight: bool,
    reviewed: bool,
}

struct AuditSession {
    campaign_id: String,
    launch_token: Option<String>,
    session_token: String,
    expires_at: Instant,
    entries: Vec<AuditEntry>,
    submitted: u64,
    writer: Arc<dyn AuditWriter>,
}

impl AuditSession {
    fn wipe(&mut self) {
        wipe_string(&mut self.campaign_id);
        if let Some(token) = &mut self.launch_token {
            wipe_string(token);
        }
        self.launch_token = None;
        wipe_string(&mut self.session_token);
        for entry in &mut self.entries {
            entry.bundle.wipe();
        }
        self.entries.clear();
    }
}

#[derive(Default)]
struct AuditState {
    generation: u64,
    session: Option<AuditSession>,
    server: Option<tokio::task::AbortHandle>,
}

#[derive(Clone, Default)]
pub struct QualityAuditManager {
    inner: Arc<Mutex<AuditState>>,
}

impl QualityAuditManager {
    pub async fn start(
        &self,
        pool: sqlx::SqlitePool,
        campaign_id: String,
    ) -> Result<AuditStart, String> {
        let writer = Arc::new(SqliteAuditWriter {
            pool,
            campaign_id: campaign_id.clone(),
        });
        self.start_inner(writer, campaign_id, AUDIT_TTL).await
    }

    async fn start_inner(
        &self,
        writer: Arc<dyn AuditWriter>,
        campaign_id: String,
        ttl: Duration,
    ) -> Result<AuditStart, String> {
        self.clear();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| format!("audit bind failed: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("audit address failed: {error}"))?;
        let launch_token = uuid::Uuid::new_v4().simple().to_string();
        let session_token = uuid::Uuid::new_v4().simple().to_string();
        let generation = {
            let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
            state.generation = state.generation.wrapping_add(1);
            let generation = state.generation;
            state.session = Some(AuditSession {
                campaign_id,
                launch_token: Some(launch_token.clone()),
                session_token,
                expires_at: Instant::now() + ttl,
                entries: Vec::new(),
                submitted: 0,
                writer,
            });
            generation
        };

        let app = Router::new()
            .route("/audit/{token}", get(launch))
            .route("/audit/session/{token}", get(session_page))
            .route("/audit/review", post(review))
            .with_state(self.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .server = Some(task.abort_handle());

        let expiry = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(ttl).await;
            expiry.clear_generation(generation);
        });

        Ok(AuditStart {
            url: format!("http://{address}/audit/{launch_token}"),
        })
    }

    pub fn offer_fixture(&self, quality_campaign_id: &str, bundle: FixtureAuditBundle) -> bool {
        let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let Some(session) = state.session.as_mut() else {
            return false;
        };
        if Instant::now() >= session.expires_at || session.campaign_id != quality_campaign_id {
            return false;
        }
        if session
            .entries
            .iter()
            .any(|entry| entry.bundle.fixture_id == bundle.fixture_id)
        {
            return false;
        }
        let bucket_count = session
            .entries
            .iter()
            .filter(|entry| entry.bundle.bucket == bundle.bucket)
            .count();
        if bucket_count < CASES_PER_BUCKET {
            session.entries.push(AuditEntry {
                bundle,
                in_flight: false,
                reviewed: false,
            });
            return true;
        }

        let current_coverage = tag_coverage(&session.entries, None, None);
        let replacement = session
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                entry.bundle.bucket == bundle.bucket && !entry.reviewed && !entry.in_flight
            })
            .map(|(index, _)| {
                let coverage = tag_coverage(&session.entries, Some(index), Some(&bundle));
                (index, coverage)
            })
            .filter(|(_, coverage)| *coverage > current_coverage)
            .max_by_key(|(index, coverage)| (*coverage, std::cmp::Reverse(*index)));
        if let Some((index, _)) = replacement {
            session.entries[index] = AuditEntry {
                bundle,
                in_flight: false,
                reviewed: false,
            };
            true
        } else {
            false
        }
    }

    pub fn clear(&self) {
        let (server, mut session) = {
            let mut state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
            (state.server.take(), state.session.take())
        };
        if let Some(session) = &mut session {
            session.wipe();
        }
        if let Some(server) = server {
            server.abort();
        }
    }

    fn clear_generation(&self, generation: u64) {
        let should_clear = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generation
            == generation;
        if should_clear {
            self.clear();
        }
    }

    pub fn status(&self) -> AuditStatus {
        let state = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let Some(session) = state.session.as_ref() else {
            return AuditStatus {
                active: false,
                selected: 0,
                submitted: 0,
                expires_in_seconds: 0,
            };
        };
        AuditStatus {
            active: Instant::now() < session.expires_at,
            selected: session.entries.len() as u64,
            submitted: session.submitted,
            expires_in_seconds: session
                .expires_at
                .saturating_duration_since(Instant::now())
                .as_secs(),
        }
    }

    #[cfg(test)]
    fn raw_bytes(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .map(|entry| entry.bundle.raw_bytes())
                    .sum()
            })
            .unwrap_or(0)
    }
}

fn tag_coverage(
    entries: &[AuditEntry],
    skip: Option<usize>,
    replacement: Option<&FixtureAuditBundle>,
) -> usize {
    let mut tags = std::collections::HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if Some(index) != skip {
            tags.extend(entry.bundle.tags.iter().copied());
        }
    }
    if let Some(bundle) = replacement {
        tags.extend(bundle.tags.iter().copied());
    }
    tags.len()
}

#[derive(Deserialize)]
struct ReviewForm {
    session_token: String,
    case_id: String,
    verdict: String,
}

async fn launch(State(manager): State<QualityAuditManager>, Path(token): Path<String>) -> Response {
    let body = {
        let mut state = manager
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(session) = state.session.as_mut() else {
            return secure_response(StatusCode::NOT_FOUND, "Audit unavailable".into());
        };
        if Instant::now() >= session.expires_at || session.launch_token.as_deref() != Some(&token) {
            return secure_response(StatusCode::NOT_FOUND, "Audit unavailable".into());
        }
        if let Some(mut launch_token) = session.launch_token.take() {
            wipe_string(&mut launch_token);
        }
        render_session(session, None)
    };
    secure_response(StatusCode::OK, body)
}

async fn session_page(
    State(manager): State<QualityAuditManager>,
    Path(token): Path<String>,
) -> Response {
    let state = manager
        .inner
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let Some(session) = state.session.as_ref() else {
        return secure_response(StatusCode::NOT_FOUND, "Audit unavailable".into());
    };
    if Instant::now() >= session.expires_at || session.session_token != token {
        return secure_response(StatusCode::NOT_FOUND, "Audit unavailable".into());
    }
    secure_response(StatusCode::OK, render_session(session, None))
}

async fn review(
    State(manager): State<QualityAuditManager>,
    Form(form): Form<ReviewForm>,
) -> Response {
    let verdict = match form.verdict.as_str() {
        "agree" => AuditVerdict::Agree,
        "disagree" => AuditVerdict::Disagree,
        _ => return secure_response(StatusCode::BAD_REQUEST, "Invalid verdict".into()),
    };
    let (generation, writer, bucket, label_a_is_original) = {
        let mut state = manager
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let generation = state.generation;
        let Some(session) = state.session.as_mut() else {
            return secure_response(StatusCode::NOT_FOUND, "Audit unavailable".into());
        };
        if Instant::now() >= session.expires_at || session.session_token != form.session_token {
            return secure_response(StatusCode::NOT_FOUND, "Audit unavailable".into());
        }
        let Some(entry) = session.entries.iter_mut().find(|entry| {
            entry.bundle.case_id == form.case_id && !entry.reviewed && !entry.in_flight
        }) else {
            return secure_response(StatusCode::CONFLICT, "Verdict already submitted".into());
        };
        entry.in_flight = true;
        (
            generation,
            session.writer.clone(),
            entry.bundle.bucket,
            entry.bundle.label_a_is_original,
        )
    };

    if let Err(error) = writer.write(&form.case_id, bucket, verdict).await {
        let mut state = manager
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.generation == generation {
            if let Some(entry) = state.session.as_mut().and_then(|session| {
                session
                    .entries
                    .iter_mut()
                    .find(|entry| entry.bundle.case_id == form.case_id)
            }) {
                entry.in_flight = false;
            }
        }
        return secure_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Audit persistence failed: {}", escape_html(&error)),
        );
    }

    let (body, complete) = {
        let mut state = manager
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.generation != generation {
            return secure_response(StatusCode::CONFLICT, "Audit session changed".into());
        }
        let session = state.session.as_mut().expect("generation owns a session");
        let entry = session
            .entries
            .iter_mut()
            .find(|entry| entry.bundle.case_id == form.case_id)
            .expect("submitted entry remains selected");
        entry.in_flight = false;
        entry.reviewed = true;
        session.submitted += 1;
        let reveal = if label_a_is_original {
            "Recorded. Plan A was original; Plan B was projected."
        } else {
            "Recorded. Plan A was projected; Plan B was original."
        };
        let complete = session.submitted >= MAX_AUDIT_CASES as u64;
        (render_session(session, Some(reveal)), complete)
    };
    if complete {
        let cleanup = manager.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cleanup.clear_generation(generation);
        });
    }
    secure_response(StatusCode::OK, body)
}

fn render_session(session: &AuditSession, reveal: Option<&str>) -> String {
    let mut body = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Conclave H2 audit</title>\
         <style>body{font:16px system-ui;max-width:960px;margin:2rem auto;padding:0 1rem}\
         pre{white-space:pre-wrap;background:#f3f4f6;padding:1rem;border-radius:.5rem}\
         button{margin-right:.75rem;padding:.6rem 1rem}</style></head><body>",
    );
    body.push_str("<h1>Conclave H2 synthetic audit</h1>");
    if let Some(reveal) = reveal {
        body.push_str("<p><strong>");
        body.push_str(&escape_html(reveal));
        body.push_str("</strong></p>");
    }
    let next = session
        .entries
        .iter()
        .find(|entry| !entry.reviewed && !entry.in_flight);
    if let Some(entry) = next {
        let bundle = &entry.bundle;
        let (plan_a, plan_b) = if bundle.label_a_is_original {
            (&bundle.original_plan, &bundle.projected_plan)
        } else {
            (&bundle.projected_plan, &bundle.original_plan)
        };
        body.push_str(&format!(
            "<p>Case {}/12 · bucket <code>{}</code> · fixture <code>{}</code></p>",
            session.submitted + 1,
            bundle.bucket.as_str(),
            escape_html(&bundle.fixture_id),
        ));
        for (title, value) in [
            ("Synthetic source", &bundle.source_json),
            ("Synthetic summary", &bundle.summary),
            ("Generated probes", &bundle.probes),
            ("Verifier output", &bundle.verifier),
            ("Plan A", plan_a),
            ("Plan B", plan_b),
            ("Judge A", &bundle.judge_a),
            ("Judge B", &bundle.judge_b),
        ] {
            body.push_str("<h2>");
            body.push_str(title);
            body.push_str("</h2><pre>");
            body.push_str(&escape_html(value));
            body.push_str("</pre>");
        }
        body.push_str("<form method=\"post\" action=\"/audit/review\">");
        body.push_str(&format!(
            "<input type=\"hidden\" name=\"session_token\" value=\"{}\">\
             <input type=\"hidden\" name=\"case_id\" value=\"{}\">",
            escape_html(&session.session_token),
            escape_html(&bundle.case_id),
        ));
        body.push_str(
            "<button name=\"verdict\" value=\"agree\">Agree</button>\
             <button name=\"verdict\" value=\"disagree\">Disagree</button></form>",
        );
    } else if session.submitted >= MAX_AUDIT_CASES as u64 {
        body.push_str("<p>Audit complete. Raw fixture material has been cleared.</p>");
    } else {
        body.push_str("<p>Waiting for completed synthetic fixture cases.</p>");
        body.push_str(&format!(
            "<p><a href=\"/audit/session/{}\">Refresh</a></p>",
            escape_html(&session.session_token)
        ));
    }
    body.push_str("</body></html>");
    body
}

fn secure_response(status: StatusCode, body: String) -> Response {
    let mut response = (status, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[derive(Default)]
    struct MockWriter {
        writes: Mutex<HashSet<String>>,
    }

    impl AuditWriter for MockWriter {
        fn write(
            &self,
            case_id: &str,
            _bucket: AuditBucket,
            _verdict: AuditVerdict,
        ) -> AuditWriteFuture<'_> {
            let case_id = case_id.to_owned();
            Box::pin(async move {
                if self.writes.lock().unwrap().insert(case_id) {
                    Ok(())
                } else {
                    Err("duplicate".into())
                }
            })
        }
    }

    fn bundle(index: usize, bucket: AuditBucket, tags: Vec<QualityTag>) -> FixtureAuditBundle {
        FixtureAuditBundle {
            case_id: format!("00000000-0000-0000-0000-{index:012}"),
            fixture_id: format!("fixture-{index:02}"),
            tags,
            bucket,
            source_json: "{\"source\":\"raw\"}".into(),
            summary: "synthetic summary".into(),
            probes: "[]".into(),
            original_plan: "original plan".into(),
            projected_plan: "projected plan".into(),
            verifier: "{\"probeRecall\":1}".into(),
            judge_a: "{\"correct\":true}".into(),
            judge_b: "{\"correct\":false}".into(),
            label_a_is_original: index.is_multiple_of(2),
        }
    }

    async fn started(ttl: Duration) -> (QualityAuditManager, AuditStart, Arc<MockWriter>) {
        let manager = QualityAuditManager::default();
        let writer = Arc::new(MockWriter::default());
        let start = manager
            .start_inner(writer.clone(), "campaign".into(), ttl)
            .await
            .unwrap();
        (manager, start, writer)
    }

    #[tokio::test]
    async fn reservoir_selects_four_per_bucket_and_covers_all_tags() {
        let (manager, _start, _writer) = started(Duration::from_secs(60)).await;
        for (index, tag) in QualityTag::ALL.into_iter().enumerate() {
            let bucket = match index % 3 {
                0 => AuditBucket::Accepted,
                1 => AuditBucket::Rejected,
                _ => AuditBucket::NearThreshold,
            };
            assert!(manager.offer_fixture("campaign", bundle(index, bucket, vec![tag])));
        }
        for index in 7..12 {
            let bucket = match index % 3 {
                0 => AuditBucket::Accepted,
                1 => AuditBucket::Rejected,
                _ => AuditBucket::NearThreshold,
            };
            manager.offer_fixture("campaign", bundle(index, bucket, vec![QualityTag::LongLog]));
        }
        let state = manager.inner.lock().unwrap();
        let session = state.session.as_ref().unwrap();
        assert_eq!(session.entries.len(), 12);
        for bucket in [
            AuditBucket::Accepted,
            AuditBucket::Rejected,
            AuditBucket::NearThreshold,
        ] {
            assert_eq!(
                session
                    .entries
                    .iter()
                    .filter(|entry| entry.bundle.bucket == bucket)
                    .count(),
                4
            );
        }
        assert_eq!(
            tag_coverage(&session.entries, None, None),
            QualityTag::ALL.len()
        );
    }

    #[tokio::test]
    async fn loopback_page_has_strict_headers_one_use_launch_and_one_shot_verdict() {
        let (manager, start, writer) = started(Duration::from_secs(60)).await;
        manager.offer_fixture(
            "campaign",
            bundle(1, AuditBucket::Accepted, vec![QualityTag::ExactError]),
        );
        let client = reqwest::Client::new();
        let first = client.get(&start.url).send().await.unwrap();
        assert_eq!(first.status(), reqwest::StatusCode::OK);
        assert_eq!(first.headers()[header::CACHE_CONTROL], "no-store");
        assert!(first.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("default-src 'none'"));
        assert_eq!(first.headers()[header::REFERRER_POLICY], "no-referrer");
        let blinded = first.text().await.unwrap();
        assert!(!blinded.contains("was original"));
        assert!(!blinded.contains("was projected"));
        let repeated = client.get(&start.url).send().await.unwrap();
        assert_eq!(repeated.status(), reqwest::StatusCode::NOT_FOUND);

        let (token, case_id) = {
            let state = manager.inner.lock().unwrap();
            let session = state.session.as_ref().unwrap();
            (
                session.session_token.clone(),
                session.entries[0].bundle.case_id.clone(),
            )
        };
        let origin = start.url.split("/audit/").next().unwrap();
        let submit = client
            .post(format!("{origin}/audit/review"))
            .form(&[
                ("session_token", token.as_str()),
                ("case_id", case_id.as_str()),
                ("verdict", "agree"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(submit.status(), reqwest::StatusCode::OK);
        assert!(submit.text().await.unwrap().contains("Plan A was"));
        let duplicate = client
            .post(format!("{origin}/audit/review"))
            .form(&[
                ("session_token", token.as_str()),
                ("case_id", case_id.as_str()),
                ("verdict", "agree"),
            ])
            .send()
            .await
            .unwrap();
        assert_eq!(duplicate.status(), reqwest::StatusCode::CONFLICT);
        assert_eq!(writer.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn stop_and_timeout_drop_all_raw_fixture_material() {
        let (manager, _start, _writer) = started(Duration::from_millis(25)).await;
        manager.offer_fixture(
            "campaign",
            bundle(1, AuditBucket::Accepted, vec![QualityTag::LongLog]),
        );
        assert!(manager.raw_bytes() > 0);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(manager.raw_bytes(), 0);
        assert!(!manager.status().active);

        let start = manager
            .start_inner(
                Arc::new(MockWriter::default()),
                "campaign".into(),
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        manager.offer_fixture(
            "campaign",
            bundle(2, AuditBucket::Rejected, vec![QualityTag::ExactError]),
        );
        assert!(manager.raw_bytes() > 0);
        manager.clear();
        assert_eq!(manager.raw_bytes(), 0);
        assert!(!manager.status().active);
        drop(start);
    }

    #[test]
    fn completed_bundle_uses_the_manifest_bucket_and_has_no_live_constructor() {
        let scores = JudgeScores {
            correct: true,
            constraint_adherent: true,
            next_action_match: true,
        };
        for (fixture_id, expected) in [
            ("side-effecting-output-01", AuditBucket::Accepted),
            ("exact-error-01", AuditBucket::Rejected),
            ("rejected-alternative-01", AuditBucket::NearThreshold),
        ] {
            let bundle = FixtureAuditBundle::completed(
                uuid::Uuid::new_v4().to_string(),
                fixture_id,
                vec![QualityTag::ExactError],
                "[]".into(),
                "summary".into(),
                &[],
                "original".into(),
                "projected".into(),
                "{}".into(),
                scores,
                scores,
                true,
            )
            .unwrap();
            assert_eq!(bundle.bucket, expected);
        }
    }

    #[tokio::test]
    async fn twelfth_submission_completes_and_zeroizes_the_session() {
        let (manager, start, writer) = started(Duration::from_secs(60)).await;
        for index in 0..12 {
            let bucket = match index % 3 {
                0 => AuditBucket::Accepted,
                1 => AuditBucket::Rejected,
                _ => AuditBucket::NearThreshold,
            };
            assert!(manager.offer_fixture(
                "campaign",
                bundle(index, bucket, vec![QualityTag::ALL[index % QualityTag::ALL.len()]])
            ));
        }
        let client = reqwest::Client::new();
        assert_eq!(
            client.get(&start.url).send().await.unwrap().status(),
            reqwest::StatusCode::OK
        );
        let origin = start.url.split("/audit/").next().unwrap();
        for index in 0..12 {
            let (token, case_id) = {
                let state = manager.inner.lock().unwrap();
                let session = state.session.as_ref().unwrap();
                (
                    session.session_token.clone(),
                    session.entries[index].bundle.case_id.clone(),
                )
            };
            let response = client
                .post(format!("{origin}/audit/review"))
                .form(&[
                    ("session_token", token.as_str()),
                    ("case_id", case_id.as_str()),
                    ("verdict", "agree"),
                ])
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::OK);
        }
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!manager.status().active);
        assert_eq!(manager.raw_bytes(), 0);
        assert_eq!(writer.writes.lock().unwrap().len(), 12);
    }
}
