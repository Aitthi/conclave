//! Task stall + challenge-default timer (ADR 0008 Lane B).
//!
//! A single app-wide background loop ([`run`], spawned once at startup next
//! to the UDS server — see `lib.rs`'s `.setup()`) that every
//! [`TICK_INTERVAL`]:
//!
//! 1. **Stall check**: a `claimed`/`in_progress` task whose newest
//!    `task_event` is [`STALL_MINUTES`]+ old gets its OWNER notified, at most
//!    once per [`STALL_ALERT_COOLDOWN_MINUTES`] per task (tracked
//!    in-process via [`Ticker`], not the DB — a missed alert across an app
//!    restart is fine; a duplicate every tick is not).
//! 2. **Challenge-default check**: a `challenge` event whose stored
//!    `deadlineAt` (an absolute ISO instant, computed at insert time — see
//!    `commands::task::challenge`) has passed with no matching `ruling`
//!    event gets a default ruling inserted (`payload.by = "default"`,
//!    `text` = the challenge's own `--default` action) and both the
//!    challenge's actor and the task's owner notified.
//!
//! [`tick`] is the pure(-ish) async core — it takes `now` as a parameter
//! rather than reading the clock itself, so tests drive stall/deadline
//! boundary behavior deterministically instead of waiting on a real 5-minute
//! interval.
//!
//! # notify path
//!
//! Every notification goes through `commands::message::inject` — the SAME
//! delivery path `tell` uses (ADR 0008 risk ledger: "do not invent a second
//! injection path"). `inter_agent_message.from_instance_id` is a `NOT NULL`
//! FK to a real `workspace_agent`, so there is no "system" sender available;
//! both notify paths here attribute the message to a REAL party already
//! involved with the task (implementer for a stall alert to the owner; the
//! challenge's own actor/owner for a challenge-default, notifying each other)
//! rather than fabricating a fake identity.

use crate::engine::{commands, repo, AppState};
use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use std::collections::{HashMap, HashSet};

/// Production tick interval (ADR 0008 plan: "every 5 min").
const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// A task counts as stalled once its newest `task_event` is this many
/// minutes old. Chosen at 10 (plan `watch-filter`, decision 3): the human
/// asked for 5-10, and 10 clears the 5-8 quiet minutes a full `cargo
/// test`/`clippy` gate legitimately runs, so a normal build never false-pages.
/// Since the watch fan-out now injects only decision-demanding events, this
/// stall page is the safety net that pulls the lead in to CHECK a quiet claim
/// carrying an important-but-unmarked note.
const STALL_MINUTES: i64 = 10;

/// A stalled task's owner is alerted at most once per this many minutes.
const STALL_ALERT_COOLDOWN_MINUTES: i64 = 30;

/// Per-task last-stall-alert timestamps, held across ticks by [`run`]'s loop.
/// Deliberately in-process only (ADR 0008 plan: "track last-alert in memory,
/// not DB") — restarting the app resets the cooldown, which is an acceptable
/// trade for not persisting a purely advisory rate-limit.
pub struct Ticker {
    last_stall_alert: HashMap<String, DateTime<Utc>>,
}

impl Ticker {
    pub fn new() -> Self {
        Self {
            last_stall_alert: HashMap::new(),
        }
    }
}

impl Default for Ticker {
    fn default() -> Self {
        Self::new()
    }
}

/// Run the timer forever: `tick` every [`TICK_INTERVAL`] against the real
/// clock. Spawned once at app startup (`lib.rs`) — never call this from a
/// test; call [`tick`] directly with an injected `now` instead.
pub async fn run(state: std::sync::Arc<AppState>) {
    let mut ticker = Ticker::new();
    loop {
        tick(&state, Utc::now(), &mut ticker).await;
        tokio::time::sleep(TICK_INTERVAL).await;
    }
}

/// One timer pass: stall check then challenge-default check. `now` is
/// injected so tests can drive exact boundary behavior.
pub async fn tick(state: &AppState, now: DateTime<Utc>, ticker: &mut Ticker) {
    check_stalls(state, now, ticker).await;
    check_challenge_deadlines(state, now).await;
}

/// Deliver one notify line via the shared `tell` mechanism. Best-effort: the
/// caller ignores failures (an offline/unknown recipient must never break
/// the timer loop).
async fn notify(state: &AppState, from_instance_id: &str, to_instance_id: &str, text: &str) {
    let _ = commands::message::inject(
        state,
        json!({ "fromInstanceId": from_instance_id, "toInstanceId": to_instance_id, "text": text }),
    )
    .await;
}

async fn check_stalls(state: &AppState, now: DateTime<Utc>, ticker: &mut Ticker) {
    let Ok(candidates) = repo::task::stall_candidates(&state.db).await else {
        return;
    };

    for c in candidates {
        let Ok(last_event) = DateTime::parse_from_rfc3339(&c.last_event_at) else {
            continue;
        };
        let last_event = last_event.with_timezone(&Utc);
        let stale_for = now.signed_duration_since(last_event);
        if stale_for < Duration::minutes(STALL_MINUTES) {
            continue;
        }

        if let Some(last_alert) = ticker.last_stall_alert.get(&c.id) {
            if now.signed_duration_since(*last_alert) < Duration::minutes(STALL_ALERT_COOLDOWN_MINUTES) {
                continue;
            }
        }

        // A claimed/in_progress task always has an implementer (`claim` sets
        // it atomically with the state move) — but the owner is optional
        // (nobody assigned one at `create`), so there may be no one to alert.
        let (Some(owner), Some(implementer)) = (&c.owner_agent_id, &c.implementer_agent_id) else {
            continue;
        };

        // "AUTO" is load-bearing (RULED 2026-07-04, Detoro): the message is
        // attributed to a real agent (`implementer`) who did NOT type it —
        // the marker stops the recipient mistaking a machine-generated ping
        // for something the implementer actually wrote.
        let text = format!(
            "[task {}] AUTO stall alert — no activity for {}+ min (state={})",
            c.slug,
            stale_for.num_minutes(),
            c.state
        );
        notify(state, implementer, owner, &text).await;
        ticker.last_stall_alert.insert(c.id, now);
    }
}

async fn check_challenge_deadlines(state: &AppState, now: DateTime<Utc>) {
    let Ok(candidates) = repo::task::open_challenge_candidates(&state.db).await else {
        return;
    };
    let Ok(payloads) = repo::task::ruling_payloads(&state.db).await else {
        return;
    };
    let ruled_ids: HashSet<String> = payloads
        .iter()
        .filter_map(|p| serde_json::from_str::<serde_json::Value>(p).ok())
        .filter_map(|v| {
            v.get("challengeId")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .collect();

    for c in candidates {
        if ruled_ids.contains(&c.event_id) {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&c.payload) else {
            continue;
        };
        // Advisory challenge (no --deadline-min given) — never auto-defaults.
        let Some(deadline_at_str) = payload.get("deadlineAt").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Ok(deadline_at) = DateTime::parse_from_rfc3339(deadline_at_str) else {
            continue;
        };
        if now < deadline_at.with_timezone(&Utc) {
            continue;
        }

        let default_action = payload
            .get("default")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let ruling_payload = json!({
            "challengeId": c.event_id,
            "text": default_action,
            "by": "default",
        })
        .to_string();

        // Insert the default ruling — actor_agent_id is None: no real agent
        // triggered this, the payload's "by":"default" marker IS the
        // attribution (ADR 0008: "record ruling{by:\"default\"}").
        if repo::task::add_ruling(&state.db, &c.workspace_id, &c.slug, None, &ruling_payload)
            .await
            .is_err()
        {
            continue;
        }

        // task:changed (RULED 2026-07-04, Detoro): a default ruling doesn't
        // go through `commands::task`'s handlers (it bypasses `emit_changed`
        // by construction — the whole point is that no agent called
        // `task.rule`), so the LaneBoard would otherwise never see the
        // open->ruled chip flip until unrelated activity touched the task.
        if let Ok(Some(task)) = repo::task::get(&state.db, &c.workspace_id, &c.slug).await {
            state.emit(
                crate::engine::bus::TASK_CHANGED,
                crate::engine::bus::TaskChanged {
                    workspace_id: task.workspace_id,
                    task_id: task.id,
                    slug: task.slug,
                    state: task.state,
                },
            );
        }

        // "AUTO" is load-bearing (RULED 2026-07-04, Detoro) — same rationale
        // as the stall alert: the swapped-sender attribution below is a REAL
        // party to the challenge, but neither actually typed this line.
        let text = format!("[task {}] AUTO default ruling — {default_action}", c.slug);
        match (&c.actor_agent_id, &c.owner_agent_id) {
            (Some(actor), Some(owner)) if actor != owner => {
                notify(state, owner, actor, &text).await;
                notify(state, actor, owner, &text).await;
            }
            (Some(actor), _) => notify(state, actor, actor, &text).await,
            (None, Some(owner)) => notify(state, owner, owner, &text).await,
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::commands::task;
    use crate::engine::repo::{
        agent_definition::{self, AgentDefinitionInput},
        workspace, workspace_agent,
    };
    use serde_json::json;

    async fn fixture_workspace(state: &AppState) -> String {
        workspace::create(&state.db, "WS", "/tmp/ws", None)
            .await
            .expect("create workspace failed")
            .id
    }

    async fn fixture_instance(state: &AppState, workspace_id: &str, name: &str) -> String {
        let def = agent_definition::create(
            &state.db,
            &AgentDefinitionInput {
                name: name.into(),
                agent_type: "cli".into(),
                harness_mode: "own".into(),
                ..Default::default()
            },
        )
        .await
        .expect("create agent_def failed");
        workspace_agent::instantiate(&state.db, workspace_id, &def.id)
            .await
            .expect("instantiate failed")
            .id
    }

    // ── stall detection ─────────────────────────────────────────────────

    #[tokio::test]
    async fn stall_alert_fires_after_threshold_and_notifies_owner() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let implementer = fixture_instance(&state, &ws, "Implementer").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        task::claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer }))
            .await
            .expect("claim failed");

        let claimed_at = Utc::now();
        let mut ticker = Ticker::new();

        // Just under the 10-minute threshold — no alert yet.
        tick(&state, claimed_at + Duration::minutes(9), &mut ticker).await;
        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        assert_eq!(inbox.as_array().unwrap().len(), 0, "must not fire before the threshold");

        // Past the threshold — alert fires.
        tick(&state, claimed_at + Duration::minutes(11), &mut ticker).await;
        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        let arr = inbox.as_array().unwrap();
        assert_eq!(arr.len(), 1, "must fire once past the threshold");
        let text = arr[0]["text"].as_str().unwrap();
        assert!(
            text.contains("AUTO"),
            "line must carry the machine-generated marker (RULED 2026-07-04): {text}"
        );
        assert!(text.contains("stall"), "line must say it's a stall alert: {text}");
        assert!(text.contains("t1"), "line must name the task: {text}");
        assert_eq!(arr[0]["fromInstanceId"], json!(implementer));
        assert_eq!(arr[0]["toInstanceId"], json!(owner));
    }

    #[tokio::test]
    async fn stall_alert_respects_the_cooldown() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let implementer = fixture_instance(&state, &ws, "Implementer").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        task::claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer }))
            .await
            .expect("claim failed");

        let claimed_at = Utc::now();
        let mut ticker = Ticker::new();
        tick(&state, claimed_at + Duration::minutes(11), &mut ticker).await;
        // 20 minutes after the first alert — still within the 30-minute
        // cooldown, no second alert.
        tick(&state, claimed_at + Duration::minutes(31), &mut ticker).await;

        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        assert_eq!(
            inbox.as_array().unwrap().len(),
            1,
            "cooldown must suppress a second alert within the cooldown window"
        );

        // Past the cooldown (31 min after the first alert) — a second fires.
        tick(&state, claimed_at + Duration::minutes(11 + 31), &mut ticker).await;
        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        assert_eq!(inbox.as_array().unwrap().len(), 2, "cooldown expired -> second alert fires");
    }

    /// The cooldown MUST be per-task, not a single global gate — Ticker keys
    /// `last_stall_alert` by task id, so one stalled task firing must never
    /// suppress a DIFFERENT stalled task's alert on the very same tick.
    #[tokio::test]
    async fn stall_cooldown_is_per_task_not_global() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let implementer = fixture_instance(&state, &ws, "Implementer").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create t1");
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t2", "title": "T2", "ownerAgentId": owner }),
        )
        .await
        .expect("create t2");
        task::claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer }))
            .await
            .expect("claim t1 failed");
        task::claim(&state, json!({ "workspaceId": ws, "slug": "t2", "actorId": implementer }))
            .await
            .expect("claim t2 failed");

        let claimed_at = Utc::now();
        let mut ticker = Ticker::new();
        // Both tasks stall together on the SAME tick.
        tick(&state, claimed_at + Duration::minutes(11), &mut ticker).await;
        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        assert_eq!(
            inbox.as_array().unwrap().len(),
            2,
            "both t1 and t2 must alert independently on the same tick — one task's cooldown \
             entry must not gate the other's"
        );

        // Within the cooldown on a second tick: BOTH stay cooled down (proves
        // the cooldown is keyed per-task, not a single last-fired-at timestamp
        // that a second task would incorrectly race past or get blocked by).
        tick(&state, claimed_at + Duration::minutes(26), &mut ticker).await;
        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        assert_eq!(
            inbox.as_array().unwrap().len(),
            2,
            "both tasks' cooldowns must independently suppress a same-cooldown-window repeat"
        );
    }

    /// A real test clock can't tell a claim-time event apart from a note
    /// fired moments later (both land within the same test's few
    /// milliseconds of real wall-clock time — no injected `now` can carve a
    /// meaningful gap between them). So this asserts the mechanism directly
    /// at the repo layer: `stall_candidates`'s `last_event_at` MUST track the
    /// newest `task_event`, not the task's original `claim`-time state event.
    #[tokio::test]
    async fn stall_candidates_last_event_at_tracks_the_newest_event_not_the_claim() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let implementer = fixture_instance(&state, &ws, "Implementer").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        task::claim(&state, json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer }))
            .await
            .expect("claim failed");
        task::note(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "actorId": implementer, "text": "still working" }),
        )
        .await
        .expect("note failed");

        let got = task::get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        let events = got["events"].as_array().unwrap();
        assert_eq!(events.len(), 2, "claim's state event + the note");
        let newest_created_at = events[0]["createdAt"].as_str().unwrap(); // DESC: [0] is newest

        let candidates = repo::task::stall_candidates(&state.db)
            .await
            .expect("stall_candidates query failed");
        let candidate = candidates.iter().find(|c| c.slug == "t1").expect("t1 present");
        assert_eq!(
            candidate.last_event_at, newest_created_at,
            "stall clock must track the NEWEST event (the note), not the original claim"
        );
    }

    #[tokio::test]
    async fn planned_task_never_stalls() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        // Never claimed — stays 'planned'.

        let mut ticker = Ticker::new();
        tick(&state, Utc::now() + Duration::hours(2), &mut ticker).await;

        let inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        assert_eq!(inbox.as_array().unwrap().len(), 0, "an unclaimed task must never stall-alert");
    }

    // ── challenge default ────────────────────────────────────────────────

    #[tokio::test]
    async fn overdue_challenge_gets_a_default_ruling_and_notifies_both_parties() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        let challenge_event = task::challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": challenger,
                "claim": "X is broken", "evidence": "log", "proposal": "fix Y",
                "default": "escalate to lead", "deadlineMin": 30
            }),
        )
        .await
        .expect("challenge failed");

        let filed_at = Utc::now();
        let mut ticker = Ticker::new();

        // Before the deadline — no default ruling yet.
        tick(&state, filed_at + Duration::minutes(20), &mut ticker).await;
        let got = task::get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        assert_eq!(
            got["events"].as_array().unwrap().len(),
            1,
            "only the challenge event so far, no ruling"
        );

        // Past the deadline — default ruling inserted.
        tick(&state, filed_at + Duration::minutes(31), &mut ticker).await;
        let got = task::get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        let events = got["events"].as_array().unwrap();
        assert_eq!(events.len(), 2, "a default ruling event must be appended");
        let ruling = events.iter().find(|e| e["kind"] == "ruling").expect("ruling present");
        assert_eq!(ruling["payload"]["by"], json!("default"));
        assert_eq!(ruling["payload"]["challengeId"], challenge_event["id"]);
        assert_eq!(ruling["payload"]["text"], json!("escalate to lead"));

        // Both parties notified — `message.list` returns inbox+outbox, and
        // each party here both SENDS one (as the "from" for the other's
        // notify) and RECEIVES one, so filter to `toInstanceId` to count
        // actual received notifications rather than raw list length.
        let owner_inbox = crate::engine::commands::message::list(&state, json!({ "instanceId": owner }))
            .await
            .expect("list failed");
        let owner_received: Vec<_> = owner_inbox
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["toInstanceId"] == json!(owner))
            .collect();
        assert_eq!(owner_received.len(), 1, "owner must receive exactly one notification");
        assert!(
            owner_received[0]["text"].as_str().unwrap().contains("AUTO"),
            "line must carry the machine-generated marker (RULED 2026-07-04): {}",
            owner_received[0]["text"]
        );

        let challenger_inbox =
            crate::engine::commands::message::list(&state, json!({ "instanceId": challenger }))
                .await
                .expect("list failed");
        let challenger_received: Vec<_> = challenger_inbox
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["toInstanceId"] == json!(challenger))
            .collect();
        assert_eq!(challenger_received.len(), 1, "challenger must receive exactly one notification");
        assert!(challenger_received[0]["text"].as_str().unwrap().contains("AUTO"));
    }

    #[tokio::test]
    async fn overdue_challenge_is_not_re_ruled_on_a_later_tick() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        task::challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": challenger,
                "claim": "c", "evidence": "e", "proposal": "p",
                "default": "d", "deadlineMin": 10
            }),
        )
        .await
        .expect("challenge failed");

        let filed_at = Utc::now();
        let mut ticker = Ticker::new();
        tick(&state, filed_at + Duration::minutes(11), &mut ticker).await;
        tick(&state, filed_at + Duration::minutes(20), &mut ticker).await;

        let got = task::get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        let rulings: Vec<_> = got["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "ruling")
            .collect();
        assert_eq!(rulings.len(), 1, "a second tick must not insert a duplicate default ruling");
    }

    #[tokio::test]
    async fn manually_ruled_challenge_is_never_defaulted() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let owner = fixture_instance(&state, &ws, "Owner").await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        task::create(
            &state,
            json!({ "workspaceId": ws, "slug": "t1", "title": "T1", "ownerAgentId": owner }),
        )
        .await
        .expect("create failed");
        let challenge_event = task::challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": challenger,
                "claim": "c", "evidence": "e", "proposal": "p",
                "default": "d", "deadlineMin": 10
            }),
        )
        .await
        .expect("challenge failed");
        // A real ruling arrives BEFORE the deadline passes.
        task::rule(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": owner,
                "challengeEventId": challenge_event["id"], "text": "resolved manually"
            }),
        )
        .await
        .expect("rule failed");

        let mut ticker = Ticker::new();
        tick(&state, Utc::now() + Duration::minutes(11), &mut ticker).await;

        let got = task::get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        let rulings: Vec<_> = got["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "ruling")
            .collect();
        assert_eq!(rulings.len(), 1, "the manual ruling must stand — no default appended");
        assert_eq!(rulings[0]["payload"]["by"], json!(owner));
    }

    #[tokio::test]
    async fn advisory_challenge_without_deadline_never_defaults() {
        let state = AppState::for_tests().await;
        let ws = fixture_workspace(&state).await;
        let challenger = fixture_instance(&state, &ws, "Challenger").await;
        task::create(&state, json!({ "workspaceId": ws, "slug": "t1", "title": "T1" }))
            .await
            .expect("create failed");
        task::challenge(
            &state,
            json!({
                "workspaceId": ws, "slug": "t1", "actorId": challenger,
                "claim": "c", "evidence": "e", "proposal": "p", "default": "d"
            }),
        )
        .await
        .expect("challenge failed");

        let mut ticker = Ticker::new();
        tick(&state, Utc::now() + Duration::hours(24), &mut ticker).await;

        let got = task::get(&state, json!({ "workspaceId": ws, "slug": "t1" }))
            .await
            .expect("get failed");
        assert!(
            got["events"].as_array().unwrap().iter().all(|e| e["kind"] != "ruling"),
            "an advisory (no deadline) challenge must never auto-default"
        );
    }
}
