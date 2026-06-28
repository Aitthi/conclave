//! Snapshot repository — a point-in-time marker on a session's rolling context
//! window (the snapshot manager, M4.1).
//!
//! A `snapshot` row records that the context window crossed a threshold (an
//! `auto` compaction trigger), or that the user/agent requested a checkpoint
//! (`manual` / `handoff`). Snapshots chain via `prev_snapshot_id` so the
//! timeline can be walked newest→oldest.
//!
//! # honesty seam (M4.1)
//!
//! There is no real provider token-usage telemetry yet, and no LLM
//! summarisation. The stored `tokens` value is a labelled ESTIMATE derived from
//! streamed output bytes (see `commands::instance`), and `summary` is a
//! deterministic honest string. For `auto`/`manual` markers `carried_forward`
//! is NULL. A `handoff` snapshot, however, carries REAL context: the agent's own
//! self-written summary, captured by the strategic-compact loop (the agent runs
//! `conclave snapshot save <text>`) — that text lands in `carried_forward` and is
//! read back verbatim after `/clear`. `diff` is still always NULL.
//!
//! # chain-builder usage
//!
//! The SELECT helpers (`list_for_session`, `get`, `latest_for_session`) use
//! chain-builder (mirroring `session::create_for_instance`). `create` uses raw
//! sqlx — the documented fallback — because chaining `prev_snapshot_id` via a
//! correlated subquery inside the INSERT can't be expressed with chain-builder's
//! value-only `insert`, and that subquery is what makes the chaining atomic.

use super::cb_err;
use chain_builder::{Order, QueryBuilder, Sqlite};
use chrono::Utc;
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

// ── Row struct ──────────────────────────────────────────────────────────────

/// Decoded row from the `snapshot` table.
///
/// `sqlx::FromRow` maps snake_case columns to snake_case fields.
/// `serde(rename_all = "camelCase")` emits camelCase JSON matching the
/// `Snapshot` interface in `src/ipc/types.ts`. Optional columns use
/// `skip_serializing_if` so an absent value is a missing key (matching the
/// `field?: T` TS types), never `null`.
///
/// `carried_forward` / `diff` hold raw stored text and are serialised as-is.
/// `carried_forward` holds the agent's handoff text on a `handoff` snapshot
/// (NULL on markers); `diff` is always NULL.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotRow {
    pub id: String,
    pub session_id: String, // → "sessionId"
    pub r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger_pct: Option<i64>, // → "triggerPct"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_snapshot_id: Option<String>, // → "prevSnapshotId"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carried_forward: Option<String>, // → "carriedForward" (real text on `handoff`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>, // NULL until M4.2
    pub created_at: String, // → "createdAt"
}

// ── Column list ──────────────────────────────────────────────────────────────

const COLS: [&str; 11] = [
    "id",
    "session_id",
    "type",
    "label",
    "summary",
    "tokens",
    "trigger_pct",
    "prev_snapshot_id",
    "carried_forward",
    "diff",
    "created_at",
];

// ── Input ────────────────────────────────────────────────────────────────────

/// Fields required to create a snapshot. `kind` must be one of
/// `"auto"` | `"manual"` | `"handoff"` (the schema CHECK enforces this).
#[derive(Debug, Clone)]
pub struct NewSnapshot {
    pub session_id: String,
    pub kind: String,
    pub label: Option<String>,
    pub summary: Option<String>,
    pub tokens: Option<i64>,
    pub trigger_pct: Option<i64>,
    pub prev_snapshot_id: Option<String>,
    /// Real carried-forward context (the agent's own handoff text) for a
    /// `handoff` snapshot. `None` for `auto`/`manual` markers, which carry only
    /// the token estimate.
    pub carried_forward: Option<String>,
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Insert a new snapshot and return the constructed row.
///
/// Generates a UUID v4 `id` and ISO-8601 UTC `created_at`. `carried_forward`
/// is persisted from `input` (the agent's handoff text for a `handoff`
/// snapshot; `None` for markers); `diff` is always NULL.
///
/// # Atomic `prev_snapshot_id` chaining (no TOCTOU)
///
/// The link to the prior snapshot is resolved INSIDE the INSERT via a correlated
/// subquery — `COALESCE(?, (SELECT id … ORDER BY created_at DESC, id DESC LIMIT
/// 1))` — so a caller-supplied `prev_snapshot_id` wins, otherwise the session's
/// current latest is chosen as part of the same statement. This is raw sqlx (the
/// documented fallback: chain-builder's `insert` can't express a subquery value)
/// and closes the read-then-insert race a separate `latest_for_session` lookup
/// would open: a manual `snapshot.create` and a concurrent auto-compact for the
/// same session can no longer read the same predecessor and fork the chain —
/// SQLite serialises the two writes, so the second INSERT's subquery sees the
/// first committed row. `RETURNING prev_snapshot_id` reports the chosen link.
/// The `id DESC` tie-break makes the choice deterministic when two snapshots
/// share a `created_at`.
pub async fn create(pool: &SqlitePool, input: NewSnapshot) -> sqlx::Result<SnapshotRow> {
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();

    let label = input.label;
    let summary = input.summary;
    let tokens = input.tokens;
    let trigger_pct = input.trigger_pct;
    let carried_forward = input.carried_forward;

    let prev_snapshot_id: Option<String> = sqlx::query_scalar(
        "INSERT INTO snapshot \
         (id, session_id, type, label, summary, tokens, trigger_pct, \
          prev_snapshot_id, carried_forward, diff, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, \
          COALESCE(?, (SELECT s.id FROM snapshot s WHERE s.session_id = ? \
                       ORDER BY s.created_at DESC, s.id DESC LIMIT 1)), \
          ?, NULL, ?) \
         RETURNING prev_snapshot_id",
    )
    .bind(&id)
    .bind(&input.session_id)
    .bind(&input.kind)
    .bind(&label)
    .bind(&summary)
    .bind(tokens)
    .bind(trigger_pct)
    .bind(&input.prev_snapshot_id)
    .bind(&input.session_id)
    .bind(&carried_forward)
    .bind(&created_at)
    .fetch_one(pool)
    .await?;

    Ok(SnapshotRow {
        id,
        session_id: input.session_id,
        r#type: input.kind,
        label,
        summary,
        tokens,
        trigger_pct,
        prev_snapshot_id,
        carried_forward,
        diff: None,
        created_at,
    })
}

/// List every snapshot for a session, newest-first by `created_at`.
pub async fn list_for_session(
    pool: &SqlitePool,
    session_id: &str,
) -> sqlx::Result<Vec<SnapshotRow>> {
    QueryBuilder::<Sqlite>::table("snapshot")
        .select(COLS)
        .where_eq("session_id", session_id)
        .order_by("created_at", Order::Desc)
        .fetch_all::<SnapshotRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch a snapshot by its primary-key `id`, or `None` if no such snapshot.
///
/// Production caller: `commands::snapshot::read` (the Timeline's by-id detail
/// read, M4.2).
pub async fn get(pool: &SqlitePool, id: &str) -> sqlx::Result<Option<SnapshotRow>> {
    QueryBuilder::<Sqlite>::table("snapshot")
        .select(COLS)
        .where_eq("id", id)
        .fetch_optional::<SnapshotRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch the newest snapshot for a session, or `None` if it has none yet.
///
/// `#[allow(dead_code)]`: `create` chains `prev_snapshot_id` atomically in-SQL,
/// and the strategic-compact loop uses the type-filtered
/// [`latest_handoff_for_session`] instead (an `auto` marker must not mask the
/// handoff). Kept (test-covered) as part of the repo's read API.
#[allow(dead_code)]
pub async fn latest_for_session(
    pool: &SqlitePool,
    session_id: &str,
) -> sqlx::Result<Option<SnapshotRow>> {
    QueryBuilder::<Sqlite>::table("snapshot")
        .select(COLS)
        .where_eq("session_id", session_id)
        .order_by("created_at", Order::Desc)
        .limit(1)
        .fetch_optional::<SnapshotRow, _>(pool)
        .await
        .map_err(cb_err)
}

/// Fetch the newest `handoff` snapshot for a session, or `None` if it has none.
///
/// The strategic-compact loop relies on this (not [`latest_for_session`]): in the
/// settle window after the agent saves its handoff, the output forwarder may fire
/// an `auto` snapshot that would otherwise become "latest" — masking the handoff
/// both for the loop's save-detection and for `snapshot.last`'s read-back, which
/// would then restore from a `carried_forward = NULL` marker. Filtering on
/// `type = 'handoff'` makes both paths see only real carried-forward content.
pub async fn latest_handoff_for_session(
    pool: &SqlitePool,
    session_id: &str,
) -> sqlx::Result<Option<SnapshotRow>> {
    QueryBuilder::<Sqlite>::table("snapshot")
        .select(COLS)
        .where_eq("session_id", session_id)
        .where_eq("type", "handoff")
        .order_by("created_at", Order::Desc)
        .limit(1)
        .fetch_optional::<SnapshotRow, _>(pool)
        .await
        .map_err(cb_err)
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{
        db::connect_in_memory,
        repo::{
            agent_definition::{self, AgentDefinitionInput},
            session, workspace, workspace_agent,
        },
    };

    /// Create a workspace → agent_definition → workspace_agent → session, and
    /// return the session id (a valid FK target for `snapshot.session_id`).
    async fn fixture_session(pool: &SqlitePool) -> String {
        let ws = workspace::create(pool, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed");
        let def = agent_definition::create(
            pool,
            &AgentDefinitionInput {
                name: "SnapshotTestAgent".into(),
                role: None,
                agent_type: "cli".into(),
                cli_kind: None,
                color: None,
                provider_id: None,
                model: None,
                harness_mode: "own".into(),
                share_blackboard: None,
                auto_submit_injected: None,
                allowed_senders: None,
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        let wa = workspace_agent::create(pool, &ws.id, &def.id, "idle")
            .await
            .expect("create workspace_agent failed");
        session::create_for_instance(pool, &wa.id)
            .await
            .expect("create session failed")
            .id
    }

    fn new_input(session_id: &str, kind: &str, tokens: i64, pct: i64) -> NewSnapshot {
        NewSnapshot {
            session_id: session_id.to_owned(),
            kind: kind.to_owned(),
            label: None,
            summary: Some(format!("{kind} snapshot")),
            tokens: Some(tokens),
            trigger_pct: Some(pct),
            prev_snapshot_id: None,
            carried_forward: None,
        }
    }

    /// create → list_for_session round-trip: fields preserved, newest-first.
    #[tokio::test]
    async fn create_then_list_newest_first() {
        let pool = connect_in_memory().await;
        let sid = fixture_session(&pool).await;

        let a = create(&pool, new_input(&sid, "manual", 100, 1))
            .await
            .expect("create A failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let b = create(&pool, new_input(&sid, "auto", 200, 90))
            .await
            .expect("create B failed");

        // Field preservation on the returned row.
        assert_eq!(a.session_id, sid);
        assert_eq!(a.r#type, "manual");
        assert_eq!(a.tokens, Some(100));
        assert_eq!(a.trigger_pct, Some(1));
        assert!(a.carried_forward.is_none());
        assert!(a.diff.is_none());
        assert!(!a.id.is_empty());
        assert!(!a.created_at.is_empty());

        let rows = list_for_session(&pool, &sid)
            .await
            .expect("list_for_session failed");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, b.id, "newest first");
        assert_eq!(rows[1].id, a.id, "older second");
    }

    /// prev chaining: B created with prev_snapshot_id = latest (=A) points at A.
    #[tokio::test]
    async fn prev_snapshot_chaining() {
        let pool = connect_in_memory().await;
        let sid = fixture_session(&pool).await;

        let a = create(&pool, new_input(&sid, "manual", 100, 1))
            .await
            .expect("create A failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let prev = latest_for_session(&pool, &sid)
            .await
            .expect("latest_for_session failed")
            .map(|s| s.id);
        assert_eq!(prev.as_deref(), Some(a.id.as_str()));

        let mut input = new_input(&sid, "manual", 150, 1);
        input.prev_snapshot_id = prev;
        let b = create(&pool, input).await.expect("create B failed");

        assert_eq!(b.prev_snapshot_id.as_deref(), Some(a.id.as_str()));
    }

    /// latest_handoff_for_session ignores a newer non-handoff marker — the race
    /// the strategic-compact settle window opens.
    #[tokio::test]
    async fn latest_handoff_skips_newer_auto() {
        let pool = connect_in_memory().await;
        let sid = fixture_session(&pool).await;

        // A handoff with real content…
        let mut handoff = new_input(&sid, "handoff", 50, 25);
        handoff.carried_forward = Some("goal: ship; next: tests".into());
        let h = create(&pool, handoff).await.expect("create handoff failed");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        // …then a NEWER auto marker (carried_forward NULL) lands on top.
        create(&pool, new_input(&sid, "auto", 190, 95))
            .await
            .expect("create auto failed");

        // latest_for_session sees the auto; latest_handoff_for_session sees the handoff.
        let any = latest_for_session(&pool, &sid).await.unwrap().unwrap();
        assert_eq!(any.r#type, "auto", "newest overall is the auto marker");
        let ho = latest_handoff_for_session(&pool, &sid)
            .await
            .unwrap()
            .expect("a handoff exists");
        assert_eq!(ho.id, h.id, "latest handoff is the real one, not the auto");
        assert_eq!(
            ho.carried_forward.as_deref(),
            Some("goal: ship; next: tests")
        );
    }

    /// get by id returns the row; an unknown id → None.
    #[tokio::test]
    async fn get_by_id_and_missing() {
        let pool = connect_in_memory().await;
        let sid = fixture_session(&pool).await;

        let row = create(&pool, new_input(&sid, "manual", 100, 1))
            .await
            .expect("create failed");

        let fetched = get(&pool, &row.id)
            .await
            .expect("get failed")
            .expect("snapshot should exist");
        assert_eq!(fetched, row);

        let missing = get(&pool, "no-such-snapshot").await.expect("get failed");
        assert!(missing.is_none(), "unknown id must yield None");
    }

    /// JSON serialization uses camelCase — locks the TS Snapshot contract, and
    /// a None optional (label) is absent (not null).
    #[tokio::test]
    async fn camel_case_contract() {
        let pool = connect_in_memory().await;
        let sid = fixture_session(&pool).await;

        let row = create(&pool, new_input(&sid, "manual", 100, 25))
            .await
            .expect("create failed");

        let json = serde_json::to_value(&row).expect("serialize failed");

        // camelCase keys present
        assert!(json.get("sessionId").is_some(), "must have sessionId");
        assert!(json.get("triggerPct").is_some(), "must have triggerPct");
        assert!(json.get("createdAt").is_some(), "must have createdAt");

        // snake_case must NOT appear
        assert!(json.get("session_id").is_none(), "no session_id");
        assert!(json.get("trigger_pct").is_none(), "no trigger_pct");
        assert!(json.get("created_at").is_none(), "no created_at");

        // prevSnapshotId is None here → key absent (not null).
        assert!(
            json.get("prevSnapshotId").is_none(),
            "prevSnapshotId absent when None"
        );
        // label is None → absent.
        assert!(json.get("label").is_none(), "label absent when None");
        // carriedForward / diff are NULL until M4.2 → absent.
        assert!(
            json.get("carriedForward").is_none(),
            "carriedForward absent"
        );
        assert!(json.get("diff").is_none(), "diff absent");
    }
}
