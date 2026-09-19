# Inject Outbox Coalescing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hold inter-agent messages aimed at the same target in a per-target outbox and deliver the whole stack as one paste + one submit, so a Claude Code receiver handles a READY-note / state-change / tell burst in a single turn.

**Architecture:** A new pure module `runtime/outbox.rs` owns the per-target stacks and the timer arithmetic (no I/O). `commands::message::inject` persists each row as `held` and pushes it; a new `flush_stack` does the PTY paste + Enter that `inject` does today. A 250 ms sweeper (`outbox::run`, same idiom as `task_timer::run`) drains due stacks. A migration widens the `status` CHECK to admit `held`; startup requeues any `held` leftovers.

**Tech Stack:** Rust (tokio, sqlx + chain-builder, tauri), SQLite migration, TypeScript/React (Tailwind), `pnpm uishot`.

**Spec:** `docs/superpowers/specs/2026-09-19-inject-outbox-coalescing-design.md`

## Global Constraints

- Constants (spec §Timer): `INITIAL_WAIT = 10s`, `RESET_WINDOW = 3s`, `EXTEND = 5s`, `MAX_ITEMS = 10`. No max hold time. No `source` field.
- `message.send` (human input to own agent) is NOT modified in any way (ruling 1).
- Human ROUTED sends (`ChatView.tsx` / `StdinBar.tsx` → another agent) go through `message.inject` and must pass `immediate: true`; the engine delivers that one message now as a single-item flush and leaves the target's stack untouched (spec ruling 5, challenge d157aa7e). CLI `tell` and system callers never set it.
- Files outside the recorded task boundary (`src/ipc/commands.ts`, `src/components/ChatView.tsx`, `src/components/StdinBar.tsx`) land as their OWN scoped commit `git commit -- <those paths>` — the boundary is immutable, the lead has ruled the widening on the task (event d157aa7e ruling).
- `SUBMIT_CR_DELAYS_MS`, the bracketed-paste envelope, and the `[from {name} · {id}] ` tag format are unchanged — only their caller moves.
- Emit `bus::MESSAGE_INJECTED` once per item at flush (UI keeps one bubble per message).
- Fixture timestamps are fixed literals (CLAUDE.md). UI copy is English.
- Logging idiom in this crate is `eprintln!("[tag] …")` (see `commands/instance.rs`); there is no `tracing`.
- Commit each task separately with `git commit -- <paths>` (shared tree — never a bare `git commit`).
- Gates before READY (record each with `conclave task gate <ws> inject-outbox-coalescing -- <cmd>`):
  `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, `cargo fmt --manifest-path src-tauri/Cargo.toml --check`, `pnpm exec tsc --noEmit` (there is no `pnpm typecheck` script), `pnpm uishot chat` (then OPEN `.shots/chat-default.png` with the Read tool).

---

### Task 1: Schema + repo — admit `held`, bulk status update, startup requeue

**Files:**
- Create: `src-tauri/src/engine/migrations/0033_inter_agent_message_held.sql`
- Modify: `src-tauri/src/engine/db.rs:356-367` (append a `< 33` block after the `< 32` block)
- Modify: `src-tauri/src/engine/repo/inter_agent_message.rs` (doc comment on `create`, two new fns, tests)

**Interfaces:**
- Produces: `pub async fn update_status(pool: &SqlitePool, ids: &[String], status: &str) -> sqlx::Result<()>`
- Produces: `pub async fn requeue_held(pool: &SqlitePool) -> sqlx::Result<u64>` (rows changed)

- [ ] **Step 1: Write the failing repo tests**

Append inside the existing `#[cfg(test)] mod tests` in `src-tauri/src/engine/repo/inter_agent_message.rs` (reuse whatever fixture helper that module already uses to create two workspace_agent rows — look at the existing `create` test above it and copy its setup verbatim):

```rust
    #[tokio::test]
    async fn create_accepts_held_status() {
        let pool = crate::engine::db::connect_in_memory().await;
        let (from, to) = two_instances(&pool).await; // existing helper in this module's tests
        let row = create(&pool, &from, &to, "hi", "held", true).await.expect("held is a legal status");
        assert_eq!(row.status, "held");
    }

    #[tokio::test]
    async fn update_status_flips_only_listed_ids() {
        let pool = crate::engine::db::connect_in_memory().await;
        let (from, to) = two_instances(&pool).await;
        let a = create(&pool, &from, &to, "a", "held", true).await.unwrap();
        let b = create(&pool, &from, &to, "b", "held", true).await.unwrap();
        let c = create(&pool, &from, &to, "c", "held", true).await.unwrap();
        update_status(&pool, &[a.id.clone(), b.id.clone()], "delivered").await.unwrap();
        let rows = list_for_instance(&pool, &to, 10).await.unwrap();
        let status_of = |id: &str| rows.iter().find(|r| r.id == id).unwrap().status.clone();
        assert_eq!(status_of(&a.id), "delivered");
        assert_eq!(status_of(&b.id), "delivered");
        assert_eq!(status_of(&c.id), "held");
    }

    #[tokio::test]
    async fn update_status_with_no_ids_is_a_noop() {
        let pool = crate::engine::db::connect_in_memory().await;
        update_status(&pool, &[], "delivered").await.expect("empty id list must not error");
    }

    #[tokio::test]
    async fn requeue_held_touches_only_held_rows() {
        let pool = crate::engine::db::connect_in_memory().await;
        let (from, to) = two_instances(&pool).await;
        let held = create(&pool, &from, &to, "h", "held", true).await.unwrap();
        let delivered = create(&pool, &from, &to, "d", "delivered", true).await.unwrap();
        let queued = create(&pool, &from, &to, "q", "queued", true).await.unwrap();
        let changed = requeue_held(&pool).await.unwrap();
        assert_eq!(changed, 1);
        let rows = list_for_instance(&pool, &to, 10).await.unwrap();
        let status_of = |id: &str| rows.iter().find(|r| r.id == id).unwrap().status.clone();
        assert_eq!(status_of(&held.id), "queued");
        assert_eq!(status_of(&delivered.id), "delivered");
        assert_eq!(status_of(&queued.id), "queued");
    }
```

If the module has no `two_instances` helper, add one next to the tests that inserts a workspace, an agent_definition and two workspace_agent rows the same way the existing `create` test does, and returns their ids.

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml inter_agent_message -- --nocapture`
Expected: compile error (`update_status`/`requeue_held` not found); `create_accepts_held_status` would fail on the CHECK constraint.

- [ ] **Step 3: Write the migration**

`src-tauri/src/engine/migrations/0033_inter_agent_message_held.sql`:

```sql
-- Inject outbox (spec docs/superpowers/specs/2026-09-19-inject-outbox-coalescing-design.md):
-- a message can now sit in a per-target outbox before delivery, status 'held'.
-- SQLite cannot ALTER a CHECK constraint, so rebuild inter_agent_message in
-- place — same columns and BOTH indexes as 0001_init.sql:188-189, all rows
-- preserved (same idiom as 0018_task_event_plan_check.sql). DROP TABLE drops
-- every index on the table; list_for_instance filters on to OR from and
-- list_for_workspace on from, so both are load-bearing (challenge by Dew,
-- 2026-09-19).
CREATE TABLE inter_agent_message_new (
    id               TEXT PRIMARY KEY,
    from_instance_id TEXT NOT NULL REFERENCES workspace_agent(id),
    to_instance_id   TEXT NOT NULL REFERENCES workspace_agent(id),
    text             TEXT NOT NULL,
    status           TEXT NOT NULL CHECK(status IN ('queued', 'delivered', 'held')),
    auto_submitted   INTEGER,
    created_at       TEXT NOT NULL
);
INSERT INTO inter_agent_message_new
  SELECT id, from_instance_id, to_instance_id, text, status, auto_submitted, created_at
  FROM inter_agent_message;
DROP TABLE inter_agent_message;
ALTER TABLE inter_agent_message_new RENAME TO inter_agent_message;
CREATE INDEX idx_inter_agent_msg_to   ON inter_agent_message(to_instance_id);
CREATE INDEX idx_inter_agent_msg_from ON inter_agent_message(from_instance_id);
```

Guard (add to the Task 1 tests so the omission cannot recur):

```rust
    #[tokio::test]
    async fn migration_0033_keeps_both_inter_agent_message_indexes() {
        let pool = crate::engine::db::connect_in_memory().await;
        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='inter_agent_message' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(names, vec!["idx_inter_agent_msg_from", "idx_inter_agent_msg_to"]);
    }
```

- [ ] **Step 4: Wire the migration in `db.rs`**

Directly after the `if version < 32 { … }` block (`src-tauri/src/engine/db.rs:356-367`) add:

```rust
    if version < 33 {
        let mut tx = connection.begin().await?;
        sqlx::raw_sql(include_str!(
            "migrations/0033_inter_agent_message_held.sql"
        ))
        .execute(&mut *tx)
        .await?;
        sqlx::raw_sql("PRAGMA user_version = 33;")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
```

Check the file for any test that asserts the final `user_version` (`migrate_0031_0032_preserves_events_and_initializes_evidence_to_null` at ~`db.rs:1897` asserts `version, 32`) and bump it to 33.

- [ ] **Step 4b: Upgrade-path test — migrate a populated v32 database, don't just insert on a fresh schema** (asked by Mellow, review of challenge 20ec4f31)

Add to the `db.rs` test module, next to `connect_at_v31_with_event` (~line 1855), mirroring its shape:

```rust
    /// Schema 32 with one delivered inter-agent message in place, so 0033's
    /// table rebuild is exercised on REAL rows — not on an empty table.
    async fn connect_at_v32_with_message() -> SqlitePool {
        let pool = connect_at_v31_with_event().await;
        let mut tx = pool.begin().await.expect("begin v32 setup");
        sqlx::raw_sql(include_str!("migrations/0032_model_usage_reconciliation.sql"))
            .execute(&mut *tx)
            .await
            .expect("apply 0032");
        sqlx::raw_sql("PRAGMA user_version = 32;")
            .execute(&mut *tx)
            .await
            .expect("set user_version = 32");
        // FK targets: copy the workspace / agent_definition / workspace_agent
        // INSERT statements VERBATIM from the retained-tables test earlier in
        // this module (the one that inserts 'wa-root' and 'wa-child', ~line 520-556).
        // <paste them here>
        sqlx::query(
            "INSERT INTO inter_agent_message \
             (id,from_instance_id,to_instance_id,text,status,auto_submitted,created_at) \
             VALUES ('im-legacy','wa-root','wa-child','legacy hello','delivered',1,'2026-09-01T00:00:00Z')",
        )
        .execute(&mut *tx)
        .await
        .expect("insert legacy message");
        tx.commit().await.expect("commit v32 setup");
        pool
    }

    /// 0033 rebuilds inter_agent_message to admit 'held': the legacy row
    /// survives byte-for-byte, 'held' becomes insertable, and BOTH indexes
    /// from 0001_init.sql:188-189 exist again (challenge 20ec4f31, Dew).
    #[tokio::test]
    async fn migrate_0032_0033_preserves_messages_admits_held_and_keeps_both_indexes() {
        let pool = connect_at_v32_with_message().await;

        migrate(&pool).await.expect("migrate to head");

        let version: i64 = sqlx::query_scalar("PRAGMA user_version").fetch_one(&pool).await.unwrap();
        assert_eq!(version, 33, "user_version must reach 33");

        let legacy: (String, String, String, String, i64, String) = sqlx::query_as(
            "SELECT from_instance_id,to_instance_id,text,status,auto_submitted,created_at \
             FROM inter_agent_message WHERE id='im-legacy'",
        )
        .fetch_one(&pool)
        .await
        .expect("legacy row survives the rebuild");
        assert_eq!(
            legacy,
            ("wa-root".into(), "wa-child".into(), "legacy hello".into(), "delivered".into(), 1, "2026-09-01T00:00:00Z".into())
        );

        sqlx::query(
            "INSERT INTO inter_agent_message \
             (id,from_instance_id,to_instance_id,text,status,auto_submitted,created_at) \
             VALUES ('im-held','wa-root','wa-child','held hello','held',1,'2026-09-19T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("'held' is admitted by the rebuilt CHECK");

        sqlx::query(
            "INSERT INTO inter_agent_message \
             (id,from_instance_id,to_instance_id,text,status,auto_submitted,created_at) \
             VALUES ('im-bad','wa-root','wa-child','x','bogus',1,'2026-09-19T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect_err("the CHECK still rejects unknown statuses");

        let indexes: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='inter_agent_message' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(indexes, vec!["idx_inter_agent_msg_from", "idx_inter_agent_msg_to"]);
    }
```

Add `src-tauri/src/engine/db.rs` tests to the Step 6 run: `cargo test --manifest-path src-tauri/Cargo.toml migrate_0032_0033`.

- [ ] **Step 5: Add the repo functions**

In `src-tauri/src/engine/repo/inter_agent_message.rs`, update the `create` doc line to `` `status` must be one of `"queued"` | `"delivered"` | `"held"` `` and add after `create`:

```rust
/// Set `status` on every listed row in one statement. Used by the inject
/// outbox at flush time (`held` → `delivered` | `queued`). An empty `ids`
/// slice is a no-op (no round-trip, `Ok(())`).
///
/// Raw `sqlx` (not chain-builder): the `IN (…)` list is variable-length, which
/// the fluent `where_eq` chain cannot express — same documented fallback as
/// `list_for_instance`.
pub async fn update_status(pool: &SqlitePool, ids: &[String], status: &str) -> sqlx::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!("UPDATE inter_agent_message SET status = ? WHERE id IN ({placeholders})");
    let mut query = sqlx::query(&sql).bind(status);
    for id in ids {
        query = query.bind(id);
    }
    query.execute(pool).await?;
    Ok(())
}

/// Startup repair: the outbox is in-memory, so any row still `held` after a
/// restart will never be flushed. Flip them to `queued` — the same status a
/// message to an offline target already gets. Returns the number of rows changed.
pub async fn requeue_held(pool: &SqlitePool) -> sqlx::Result<u64> {
    let result = sqlx::query("UPDATE inter_agent_message SET status = 'queued' WHERE status = 'held'")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
```

(If the toolchain rejects `std::iter::repeat_n`, use `vec!["?"; ids.len()].join(",")`.)

- [ ] **Step 6: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml inter_agent_message`
Expected: all PASS, including the four new tests. Also run `cargo test --manifest-path src-tauri/Cargo.toml db::` to confirm the retained-tables test (which inserts a `queued` row at `db.rs:556`) still passes on the rebuilt table.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/migrations/0033_inter_agent_message_held.sql src-tauri/src/engine/db.rs src-tauri/src/engine/repo/inter_agent_message.rs
git commit -m "feat(db): inter_agent_message admits 'held' + bulk status update/requeue" -- src-tauri/src/engine/migrations/0033_inter_agent_message_held.sql src-tauri/src/engine/db.rs src-tauri/src/engine/repo/inter_agent_message.rs
```

---

### Task 2: `runtime/outbox.rs` — pure per-target stacks with the timer rule

**Files:**
- Create: `src-tauri/src/engine/runtime/outbox.rs`
- Modify: `src-tauri/src/engine/runtime/mod.rs:37` (add `pub mod outbox;` in alphabetical order, between `launch_common` and `provider`)

**Interfaces:**
- Produces:
  ```rust
  pub const INITIAL_WAIT: Duration; pub const RESET_WINDOW: Duration; pub const EXTEND: Duration; pub const MAX_ITEMS: usize;
  pub struct HeldItem { pub row_id: String, pub from_instance_id: String, pub sender_name: String, pub text: String }
  pub enum Push { Held, Flush(Vec<HeldItem>) }
  pub struct Outbox;  impl Outbox { pub fn new() -> Self; pub fn push(&self, to: &str, item: HeldItem, now: Instant) -> Push; pub fn take_due(&self, now: Instant) -> Vec<(String, Vec<HeldItem>)>; pub fn take_all(&self, to: &str) -> Vec<HeldItem>; pub fn tagged_line(item: &HeldItem) -> String; pub fn body(items: &[HeldItem]) -> String }
  ```
  (`run` is added in Task 3 — it needs `flush_stack`.)

- [ ] **Step 1: Write the failing unit tests**

Create `src-tauri/src/engine/runtime/outbox.rs` with the tests first (the module body comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn item(n: usize) -> HeldItem {
        HeldItem {
            row_id: format!("row-{n}"),
            from_instance_id: format!("from-{n}"),
            sender_name: format!("Sender{n}"),
            text: format!("msg {n}"),
        }
    }

    fn deadline_of(outbox: &Outbox, to: &str) -> Instant {
        outbox.deadline(to).expect("stack exists")
    }

    #[test]
    fn first_item_starts_initial_wait() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        assert!(matches!(ob.push("A", item(1), t0), Push::Held));
        assert_eq!(deadline_of(&ob, "A"), t0 + INITIAL_WAIT);
    }

    #[test]
    fn arrival_within_reset_window_resets_to_initial_wait() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        ob.push("A", item(1), t0);
        let t1 = t0 + Duration::from_secs(1);
        ob.push("A", item(2), t1);
        assert_eq!(deadline_of(&ob, "A"), t1 + INITIAL_WAIT);
    }

    #[test]
    fn arrival_after_reset_window_extends_remaining() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        ob.push("A", item(1), t0); // deadline t0+10
        let t1 = t0 + Duration::from_secs(9); // gap 9s > 3s, remaining 1s
        ob.push("A", item(2), t1);
        assert_eq!(deadline_of(&ob, "A"), t0 + INITIAL_WAIT + EXTEND); // 1s + 5s = 6s left
    }

    #[test]
    fn gap_is_measured_from_the_last_arrival_not_the_first() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        ob.push("A", item(1), t0);
        ob.push("A", item(2), t0 + Duration::from_secs(2)); // reset → t0+12
        ob.push("A", item(3), t0 + Duration::from_secs(4)); // gap 2s from item 2 → reset → t0+14
        assert_eq!(deadline_of(&ob, "A"), t0 + Duration::from_secs(4) + INITIAL_WAIT);
    }

    #[test]
    fn extend_never_lands_in_the_past() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        ob.push("A", item(1), t0);
        let late = t0 + INITIAL_WAIT + Duration::from_secs(2); // sweeper hasn't ticked yet
        ob.push("A", item(2), late);
        assert_eq!(deadline_of(&ob, "A"), late + EXTEND);
    }

    #[test]
    fn tenth_item_flushes_in_order_and_eleventh_starts_fresh() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        for n in 1..MAX_ITEMS {
            assert!(matches!(ob.push("A", item(n), t0), Push::Held));
        }
        let Push::Flush(items) = ob.push("A", item(MAX_ITEMS), t0) else {
            panic!("10th push must flush");
        };
        let ids: Vec<_> = items.iter().map(|i| i.row_id.as_str()).collect();
        let expected: Vec<String> = (1..=MAX_ITEMS).map(|n| format!("row-{n}")).collect();
        assert_eq!(ids, expected.iter().map(String::as_str).collect::<Vec<_>>());
        assert!(ob.deadline("A").is_none(), "flushed stack is gone");
        let t1 = t0 + Duration::from_secs(1);
        assert!(matches!(ob.push("A", item(11), t1), Push::Held));
        assert_eq!(deadline_of(&ob, "A"), t1 + INITIAL_WAIT, "11th item starts a new stack");
    }

    #[test]
    fn take_due_drains_only_expired_stacks() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        ob.push("A", item(1), t0);
        ob.push("B", item(2), t0 + Duration::from_secs(5));
        let due = ob.take_due(t0 + INITIAL_WAIT);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0, "A");
        assert_eq!(due[0].1.len(), 1);
        assert!(ob.deadline("A").is_none());
        assert!(ob.deadline("B").is_some(), "B is not due yet");
        assert!(ob.take_due(t0 + INITIAL_WAIT).is_empty(), "nothing else due");
    }

    #[test]
    fn stacks_are_per_target() {
        let ob = Outbox::new();
        let t0 = Instant::now();
        ob.push("A", item(1), t0);
        ob.push("B", item(2), t0);
        assert_eq!(ob.take_all("A").len(), 1);
        assert_eq!(ob.take_all("B").len(), 1);
        assert!(ob.take_all("A").is_empty());
    }

    #[test]
    fn body_tags_each_line_and_separates_with_a_blank_line() {
        let items = vec![item(1), item(2)];
        assert_eq!(Outbox::tagged_line(&items[0]), "[from Sender1 · from-1] msg 1");
        assert_eq!(
            Outbox::body(&items),
            "[from Sender1 · from-1] msg 1\n\n[from Sender2 · from-2] msg 2"
        );
    }
}
```

- [ ] **Step 2: Register the module and run to verify failure**

Add `pub mod outbox;` to `src-tauri/src/engine/runtime/mod.rs` (alphabetical, after `pub mod launch_common;`).
Run: `cargo test --manifest-path src-tauri/Cargo.toml outbox`
Expected: compile errors — `Outbox`, `HeldItem`, `Push`, constants not defined.

- [ ] **Step 3: Implement the module**

Put this ABOVE the tests in `src-tauri/src/engine/runtime/outbox.rs`:

```rust
//! Inject outbox — per-target stacks that coalesce inter-agent messages into
//! ONE paste + ONE submit (spec docs/superpowers/specs/2026-09-19-inject-outbox-coalescing-design.md).
//!
//! Pure over `Instant`: no I/O, no clock reads. `commands::message` persists
//! rows and delivers; this module only decides WHEN a target's stack is due.
//!
//! Timer rule (human ruling 2026-09-19), per target:
//! * first item → `deadline = now + INITIAL_WAIT`
//! * next item ≤ RESET_WINDOW after the previous arrival → `deadline = now + INITIAL_WAIT`
//! * next item > RESET_WINDOW after the previous arrival → `deadline = max(deadline, now) + EXTEND`
//! * MAX_ITEMS reached → flush now; the next item starts a fresh stack
//! * no maximum hold time

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const INITIAL_WAIT: Duration = Duration::from_secs(10);
pub const RESET_WINDOW: Duration = Duration::from_secs(3);
pub const EXTEND: Duration = Duration::from_secs(5);
pub const MAX_ITEMS: usize = 10;

/// One message waiting in a target's stack. `text` is RAW (no `[from …]` tag);
/// the tag is applied by [`Outbox::tagged_line`] at flush time so the persisted
/// row and the UI keep the raw text exactly as `inject` does today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldItem {
    pub row_id: String,
    pub from_instance_id: String,
    pub sender_name: String,
    pub text: String,
}

/// Result of a push: either the item is waiting, or the cap was hit and the
/// caller must deliver the returned stack NOW.
#[derive(Debug)]
pub enum Push {
    Held,
    Flush(Vec<HeldItem>),
}

struct Stack {
    items: Vec<HeldItem>,
    deadline: Instant,
    last_arrival: Instant,
}

#[derive(Default)]
pub struct Outbox {
    stacks: Mutex<HashMap<String, Stack>>,
}

impl Outbox {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push `item` onto `to`'s stack, applying the timer rule. The mutex is
    /// held only for the map mutation — never across I/O.
    pub fn push(&self, to: &str, item: HeldItem, now: Instant) -> Push {
        let mut stacks = self.stacks.lock().unwrap_or_else(|e| e.into_inner());
        let stack = stacks.entry(to.to_owned()).or_insert_with(|| Stack {
            items: Vec::with_capacity(MAX_ITEMS),
            deadline: now + INITIAL_WAIT,
            last_arrival: now,
        });
        if !stack.items.is_empty() {
            let gap = now.saturating_duration_since(stack.last_arrival);
            stack.deadline = if gap <= RESET_WINDOW {
                now + INITIAL_WAIT
            } else {
                stack.deadline.max(now) + EXTEND
            };
            stack.last_arrival = now;
        }
        stack.items.push(item);
        if stack.items.len() >= MAX_ITEMS {
            let stack = stacks.remove(to).expect("just inserted");
            return Push::Flush(stack.items);
        }
        Push::Held
    }

    /// Remove and return every stack whose deadline has passed, as
    /// `(to_instance_id, items)` pairs. Stacks not yet due are untouched.
    pub fn take_due(&self, now: Instant) -> Vec<(String, Vec<HeldItem>)> {
        let mut stacks = self.stacks.lock().unwrap_or_else(|e| e.into_inner());
        let due: Vec<String> = stacks
            .iter()
            .filter(|(_, s)| s.deadline <= now)
            .map(|(to, _)| to.clone())
            .collect();
        due.into_iter()
            .filter_map(|to| stacks.remove(&to).map(|s| (to, s.items)))
            .collect()
    }

    /// Remove and return `to`'s whole stack regardless of deadline (empty Vec
    /// when there is none). Used by tests and by nothing on the hot path.
    pub fn take_all(&self, to: &str) -> Vec<HeldItem> {
        let mut stacks = self.stacks.lock().unwrap_or_else(|e| e.into_inner());
        stacks.remove(to).map(|s| s.items).unwrap_or_default()
    }

    /// Current deadline of `to`'s stack, if any. Test/diagnostic accessor.
    pub fn deadline(&self, to: &str) -> Option<Instant> {
        let stacks = self.stacks.lock().unwrap_or_else(|e| e.into_inner());
        stacks.get(to).map(|s| s.deadline)
    }

    /// The stdin line for one item — byte-identical to the tag `inject`
    /// writes today: `[from {name} · {id}] {text}`.
    pub fn tagged_line(item: &HeldItem) -> String {
        format!(
            "[from {} · {}] {}",
            item.sender_name, item.from_instance_id, item.text
        )
    }

    /// The single paste body for a flushed stack: tagged lines, FIFO, blank-line
    /// separated so the receiver can tell the messages apart.
    pub fn body(items: &[HeldItem]) -> String {
        items
            .iter()
            .map(Self::tagged_line)
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml outbox`
Expected: 9 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/runtime/outbox.rs src-tauri/src/engine/runtime/mod.rs
git commit -m "feat(outbox): per-target inject stacks with 10s/3s/+5s/cap-10 timer rule" -- src-tauri/src/engine/runtime/outbox.rs src-tauri/src/engine/runtime/mod.rs
```

---

### Task 3: Wire it — `inject` holds, `flush_stack` delivers, sweeper drains, startup requeues

**Files:**
- Modify: `src-tauri/src/engine/state.rs` (new `outbox` field; `new()` at ~line 77 and `for_tests()` at ~line 209)
- Modify: `src-tauri/src/engine/commands/message.rs:155-300` (`inject`), plus new `flush_stack`; tests at ~825-880
- Modify: `src-tauri/src/engine/runtime/outbox.rs` (append `pub async fn run(state)` + `TICK_INTERVAL`)
- Modify: `src-tauri/src/lib.rs:129` (spawn the sweeper right after the task timer spawn)

**Interfaces:**
- Consumes: Task 1 `repo::inter_agent_message::{update_status, requeue_held}`; Task 2 `Outbox`, `HeldItem`, `Push`.
- Produces: `pub async fn flush_stack(state: &AppState, to_instance_id: &str, items: Vec<HeldItem>) -> &'static str` (returns the final row status: `"delivered"` or `"queued"`); `pub async fn outbox::run(state: Arc<AppState>)`.

- [ ] **Step 1: Add the `outbox` field to `AppState`**

In `src-tauri/src/engine/state.rs`, add a field after `code_cache`:

```rust
    /// Inject outbox: per-target stacks of `held` inter-agent messages waiting
    /// to be flushed as ONE paste (spec 2026-09-19-inject-outbox-coalescing).
    pub outbox: crate::engine::runtime::outbox::Outbox,
```

and initialise it in BOTH constructors (`new()` and `for_tests()`):

```rust
            outbox: crate::engine::runtime::outbox::Outbox::new(),
```

Run: `cargo build --manifest-path src-tauri/Cargo.toml` — Expected: builds.

- [ ] **Step 2: Rewrite the delivery test and add the new ones (failing)**

In `src-tauri/src/engine/commands/message.rs` tests, replace `inject_live_target_retries_submit_cr` (~line 838) with these; keep `fixture_instance_id` and the `LiveHandle::for_test_pty` setup exactly as the old test had them:

```rust
    /// Two injects inside the outbox window reach the PTY as ONE bracketed
    /// paste (both tagged lines, FIFO, blank-line separated) followed by the
    /// same escalating CR retries a single message got before — the receiver
    /// handles the whole stack in one turn.
    #[tokio::test]
    async fn stack_flushes_as_one_paste_then_retries_submit_cr() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to)
            .await
            .expect("get_by_instance failed")
            .expect("session exists")
            .id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        let first = inject(&state, json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }))
            .await
            .expect("inject accepted");
        let second = inject(&state, json!({ "fromInstanceId": from, "toInstanceId": to, "text": "again" }))
            .await
            .expect("inject accepted");
        assert_eq!(first["status"], "held");
        assert_eq!(second["status"], "held");
        assert!(rx.try_recv().is_err(), "nothing reaches the PTY while held");

        let items = state.outbox.take_all(&to);
        assert_eq!(items.len(), 2);
        let status = flush_stack(&state, &to, items).await;
        assert_eq!(status, "delivered");

        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(
            writes.len(),
            1 + SUBMIT_CR_DELAYS_MS.len(),
            "expected ONE body paste + one CR per retry slot, got {writes:?}"
        );
        let body = &writes[0];
        assert!(body.starts_with("\x1b[200~[from Sender · "), "bracketed paste opens with the first tag, got {body:?}");
        assert!(body.ends_with("] again\x1b[201~"), "paste closes after the last message, got {body:?}");
        assert!(body.contains("] hi\n\n[from Sender · "), "messages are FIFO and blank-line separated, got {body:?}");
        for cr in &writes[1..] {
            assert_eq!(cr, "\r", "every follow-up write is a bare Enter");
        }

        let rows = repo::inter_agent_message::list_for_instance(&state.db, &to, 10)
            .await
            .unwrap();
        assert!(rows.iter().all(|r| r.status == "delivered"), "flush marks every row delivered: {rows:?}");
    }

    /// Reaching MAX_ITEMS flushes synchronously inside `inject`: the 10th ack
    /// already reads `delivered` and the PTY has exactly one paste.
    #[tokio::test]
    async fn tenth_inject_flushes_synchronously() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to).await.unwrap().unwrap().id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        let mut last = Value::Null;
        for n in 1..=crate::engine::runtime::outbox::MAX_ITEMS {
            last = inject(&state, json!({ "fromInstanceId": from, "toInstanceId": to, "text": format!("m{n}") }))
                .await
                .unwrap();
        }
        assert_eq!(last["status"], "delivered");
        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(writes.len(), 1 + SUBMIT_CR_DELAYS_MS.len());
        assert!(writes[0].contains("] m1\n\n") && writes[0].ends_with("] m10\x1b[201~"));
        assert!(state.outbox.take_all(&to).is_empty(), "cap flush emptied the stack");
    }

    /// A target that is not live at flush time gets `queued` rows, no PTY
    /// write, and no event — the same outcome an offline target had before.
    #[tokio::test]
    async fn flush_against_not_live_target_marks_rows_queued() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await; // never registered → NotLive
        inject(&state, json!({ "fromInstanceId": from, "toInstanceId": to, "text": "hi" }))
            .await
            .unwrap();
        let items = state.outbox.take_all(&to);
        assert_eq!(flush_stack(&state, &to, items).await, "queued");
        let rows = repo::inter_agent_message::list_for_instance(&state.db, &to, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "queued");
    }
```

Re-target these EXISTING tests — `inject` no longer touches the PTY, so its ack is always `held`:

| Test | Change |
|------|--------|
| `inject_offline_target_queues` (~line 710) | rename `inject_offline_target_is_held`; expect `Some("held")`. The NotLive → `queued` outcome is now covered by `flush_against_not_live_target_marks_rows_queued` above. Fix the inline comment (`NotLive → queued` → `held until flush`). |
| `self_inject_deduplicates_endpoint_locks` (~line 733) | expect `Some("held")`; keep the 1 s deadlock timeout — it is still the point of the test. |
| `inject_live_target_delivers` (~line 800) | rename `inject_live_target_holds_without_writing`; expect `Some("held")` and add `assert!(rx.try_recv().is_err(), "held messages do not reach the PTY")` if the test has a receiver (if it registers a handle without one, just re-target the status). |
| `inject_non_pty_target_gets_raw_body` (~line 884) | after `inject(...)`, add `flush_stack(&state, &to, state.outbox.take_all(&to)).await;` before reading `rx`. The raw-body assertion stays as is (a non-PTY backend gets `Outbox::body` with no envelope). |
| `commands/task.rs` ~line 3931 (`arr[0]["status"] == json!("delivered")`) | expect `json!("held")`; message: `"a LIVE watcher's notify is held in the outbox, never queued as offline"`. |

`grep -n '"delivered"\|"queued"' src-tauri/src/engine/commands/message.rs` after the edits: the only remaining literals should be in `repo::inter_agent_message::create(... "delivered" ...)` fixture rows for the `list` tests and inside `flush_stack` itself.

Run: `cargo test --manifest-path src-tauri/Cargo.toml commands::message`
Expected: compile error — `flush_stack` not found.

- [ ] **Step 3: Refactor `inject` and add `flush_stack`**

In `src-tauri/src/engine/commands/message.rs`:

(a) Add the imports at the top:

```rust
use crate::engine::runtime::outbox::{HeldItem, Outbox, Push};
use std::time::Instant;
```

(b) Replace the block in `inject` that starts at `let body = format!("[from {sender} · {from_instance_id}] {text}");` and ends just before `// Persist the injection FIRST` (i.e. the whole `let status = match state.runtime.send_stdin_paste(...) { … };`) AND the persist/emit tail, with:

```rust
    // Persist FIRST as `held`: the row exists before anything can flush it, so
    // a flush that races this call can never see a missing id. `text` stays
    // RAW on the row; the `[from …]` tag is applied at flush.
    let mut row = repo::inter_agent_message::create(
        &state.db,
        &from_instance_id,
        &to_instance_id,
        &text,
        "held",
        true,
    )
    .await?;

    let item = HeldItem {
        row_id: row.id.clone(),
        from_instance_id: from_instance_id.clone(),
        sender_name: sender,
        text,
    };
    // `immediate` (human routed send from the composer — spec ruling 5) skips
    // the stack entirely: deliver THIS message now and leave whatever is
    // pending for the target untouched. Everything else joins the stack.
    let push = if immediate {
        Push::Flush(vec![item])
    } else {
        state.outbox.push(&to_instance_id, item, Instant::now())
    };

    // Release BOTH lifecycle guards before a synchronous flush:
    // `flush_stack` takes the target's guards itself and the agent mutex is
    // not re-entrant — holding them here would deadlock.
    drop(_agent_guards);
    drop(_workspace_guards);

    if let Push::Flush(items) = push {
        row.status = flush_stack(state, &to_instance_id, items).await.to_owned();
    }

    serde_json::to_value(row).map_err(|e| AppError::Internal(e.to_string()))
}
```

(d) Add the flag to `InjectReq` (top of the file, ~line 124):

```rust
struct InjectReq {
    from_instance_id: String,
    to_instance_id: String,
    text: String,
    /// `true` = deliver NOW as a single-item flush, bypassing the target's
    /// outbox stack and leaving it untouched. Set ONLY by the human's routed
    /// send in the UI (ChatView / StdinBar) — spec ruling 5. CLI `tell` and
    /// system notifications never set it.
    #[serde(default)]
    immediate: bool,
}
```

and destructure it in `inject`: `let InjectReq { from_instance_id, to_instance_id, text, immediate } = …`.

(e) Add this test next to the other new ones:

```rust
    /// A human routed send (`immediate: true`) is delivered on its own, right
    /// now — and the target's pending stack stays exactly as it was.
    #[tokio::test]
    async fn immediate_inject_delivers_now_and_leaves_the_stack_alone() {
        let state = AppState::for_tests().await;
        let from = fixture_instance_id(&state, "Sender").await;
        let to = fixture_instance_id(&state, "Target").await;
        let session_id = session::get_by_instance(&state.db, &to).await.unwrap().unwrap().id;
        let (handle, mut rx) = LiveHandle::for_test_pty(&session_id);
        assert!(state.runtime.register(&to, handle).is_some());

        inject(&state, json!({ "fromInstanceId": from, "toInstanceId": to, "text": "pending" }))
            .await
            .unwrap();
        let ack = inject(
            &state,
            json!({ "fromInstanceId": from, "toInstanceId": to, "text": "from the human", "immediate": true }),
        )
        .await
        .unwrap();
        assert_eq!(ack["status"], "delivered");

        let mut writes = Vec::new();
        while let Ok(w) = rx.try_recv() {
            writes.push(w);
        }
        assert_eq!(writes.len(), 1 + SUBMIT_CR_DELAYS_MS.len(), "one paste for the immediate message only");
        assert!(writes[0].ends_with("] from the human\x1b[201~"));
        assert!(!writes[0].contains("pending"), "the held message was NOT flushed along");

        let still_held = state.outbox.take_all(&to);
        assert_eq!(still_held.len(), 1);
        assert_eq!(still_held[0].text, "pending");
    }
```

Delete the now-unused `body`/`status` bindings and the old `repo::inter_agent_message::create(... status ...)` + emit tail. Keep the long comment about the bracketed paste — move it onto `flush_stack` below so the reasoning travels with the code.

(c) Add `flush_stack` directly after `inject`:

```rust
/// Deliver one target's flushed stack as ONE bracketed paste + the escalating
/// submit CRs, then settle every row's status. Called by the outbox sweeper
/// (`runtime::outbox::run`) and by `inject` on a cap flush.
///
/// The body goes out as ONE bracketed paste (PTY backends): a body longer than
/// the kernel's PTY input queue (macOS: 1022 bytes) reaches the TUI as several
/// reads, and Claude Code's un-bracketed burst handling keeps only the LAST
/// read (docs/superpowers/plans/2026-09-04-inject-bracketed-paste.md). Inside
/// the envelope the TUI reassembles the whole body regardless of read
/// boundaries. A TUI's Enter is CR (`\r`), and a CR inside the envelope is
/// literal content, so Enter is pressed AFTER the paste at escalating gaps
/// ([`SUBMIT_CR_DELAYS_MS`]) — at least one lands as an isolated keystroke;
/// extra Enters on an empty composer are no-ops.
///
/// Returns the final row status: `"delivered"` (PTY accepted; one
/// `message:injected` event per item) or `"queued"` (target not live, backend
/// channel closed, or no longer delivery-eligible; no PTY write, no event).
/// Never errors — a flush has no caller to hand an error to.
pub async fn flush_stack(state: &AppState, to_instance_id: &str, items: Vec<HeldItem>) -> &'static str {
    if items.is_empty() {
        return "delivered";
    }
    let ids: Vec<String> = items.iter().map(|i| i.row_id.clone()).collect();

    // Same guard order as `inject`: workspace READ, then the agent mutex, then
    // re-check eligibility under the guards so a Stop that raced us wins.
    let eligibility = match require_delivery_eligible(state, to_instance_id).await {
        Ok(e) => e,
        Err(_) => return settle_rows(state, &ids, "queued").await,
    };
    let workspace_lock = state.workspace_lifecycle_lock(&eligibility.workspace_id);
    let _workspace_guard = workspace_lock.read().await;
    let agent_lock = state.agent_lifecycle_lock(to_instance_id);
    let _agent_guard = agent_lock.lock().await;
    if require_delivery_eligible(state, to_instance_id).await.is_err() {
        return settle_rows(state, &ids, "queued").await;
    }

    let body = Outbox::body(&items);
    match state.runtime.send_stdin_paste(to_instance_id, &body) {
        Ok(()) => {
            for delay_ms in SUBMIT_CR_DELAYS_MS {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                let _ = state.runtime.send_stdin(to_instance_id, "\r");
            }
        }
        Err(StdinError::NotLive) => return settle_rows(state, &ids, "queued").await,
        Err(StdinError::Closed) => {
            eprintln!(
                "[outbox] target {to_instance_id} backend stdin channel closed; {} rows queued",
                items.len()
            );
            return settle_rows(state, &ids, "queued").await;
        }
    }

    let status = settle_rows(state, &ids, "delivered").await;
    // Persist-before-emit, one event per item so the UI keeps one bubble per
    // message ("injected from X · auto-submitted").
    let to_session_id = state.runtime.session_id(to_instance_id);
    for item in items {
        state.emit(
            bus::MESSAGE_INJECTED,
            bus::MessageInjected {
                to_instance_id: to_instance_id.to_owned(),
                to_session_id: to_session_id.clone(),
                from_instance_id: item.from_instance_id,
                text: item.text,
                auto_submitted: true,
            },
        );
    }
    status
}

/// Write the final status onto every row of a flushed stack. Logs and moves
/// on if the UPDATE fails — the PTY write (if any) already happened and the
/// sweeper must never stall on a DB hiccup.
async fn settle_rows(state: &AppState, ids: &[String], status: &'static str) -> &'static str {
    if let Err(e) = repo::inter_agent_message::update_status(&state.db, ids, status).await {
        eprintln!(
            "[outbox] update_status({status}) for {} row(s) failed: {e}",
            ids.len()
        );
    }
    status
}
```

Also update the `inject` doc comment: replace the **Auto-submit** and **Status resolution** paragraphs with:

```
/// **Outbox:** the row is persisted as `"held"` and pushed onto the target's
/// outbox stack (`runtime::outbox`). Delivery happens in [`flush_stack`] —
/// from the sweeper when the stack's deadline passes, or synchronously here
/// when the push hits `MAX_ITEMS` (the ack then already reads
/// `"delivered"`/`"queued"`). Human input (`send`) never touches the outbox.
```

- [ ] **Step 4: Add the sweeper to `outbox.rs`**

Append to `src-tauri/src/engine/runtime/outbox.rs` (above the tests):

```rust
/// Sweeper cadence. The deadline is honoured within one tick.
pub const TICK_INTERVAL: Duration = Duration::from_millis(250);

/// App-wide sweeper: requeue leftovers from a previous run, then drain due
/// stacks forever. Same spawn idiom as `task_timer::run` (see `lib.rs`).
/// Flushes run sequentially per tick — a slow PTY only delays the next tick,
/// never loses a stack.
pub async fn run(state: std::sync::Arc<crate::engine::AppState>) {
    match crate::engine::repo::inter_agent_message::requeue_held(&state.db).await {
        Ok(0) => {}
        Ok(n) => eprintln!("[outbox] requeued {n} held message(s) left over from a previous run"),
        Err(e) => eprintln!("[outbox] requeue_held failed: {e}"),
    }
    loop {
        for (to, items) in state.outbox.take_due(Instant::now()) {
            crate::engine::commands::message::flush_stack(&state, &to, items).await;
        }
        tokio::time::sleep(TICK_INTERVAL).await;
    }
}
```

- [ ] **Step 5: Spawn it in `lib.rs`**

Right after the task-timer spawn (`src-tauri/src/lib.rs:129`) add:

```rust
            // Inject outbox sweeper: flushes per-target message stacks as one
            // paste once their deadline passes (spec 2026-09-19). Requeues any
            // `held` rows a previous run left behind before its first tick.
            let outbox_state = std::sync::Arc::clone(&state);
            tauri::async_runtime::spawn(engine::runtime::outbox::run(outbox_state));
```

- [ ] **Step 6: Run the tests and the full gate set**

Run: `cargo test --manifest-path src-tauri/Cargo.toml commands::message outbox inter_agent_message`
Expected: PASS.
Run: `cargo test --manifest-path src-tauri/Cargo.toml` (whole crate) — Expected: PASS. If a `task.rs`, `task_timer.rs`, `cli.rs` or `workspace.rs` test asserted an immediate `delivered`/PTY write after `inject`, adjust it to expect `held` (they only care that `inject` was reached; they never read the PTY).
Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` and `cargo fmt --manifest-path src-tauri/Cargo.toml --check` — Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/state.rs src-tauri/src/engine/commands/message.rs src-tauri/src/engine/runtime/outbox.rs src-tauri/src/lib.rs
git commit -m "feat(inject): hold messages in the outbox, flush stacks as one paste via a 250ms sweeper" -- src-tauri/src/engine/state.rs src-tauri/src/engine/commands/message.rs src-tauri/src/engine/runtime/outbox.rs src-tauri/src/lib.rs
```

---

### Task 4: TS type + `held` badge + fixture + pixel gate

**Files:**
- Modify: `src/ipc/types.ts:196`
- Modify: `src/components/ChatRail.tsx:303-305`
- Modify: `src/components/ChatHub.tsx:251-253` and `:317`
- Modify: `src/fixtures/scenarios/data.ts` (append one message to `messages`, ~line 852, BEFORE the closing `];`)

**Interfaces:**
- Consumes: the wire status `"held"` produced by Task 3.

- [ ] **Step 1: Widen the type**

`src/ipc/types.ts:196`: `status: "queued" | "delivered" | "held";`

Run: `pnpm typecheck` — Expected: clean (no consumer narrows on the union exhaustively; if one does, the error names it — add a `held` branch mirroring `queued`).

- [ ] **Step 2: Add a `held` fixture message**

Append to the `messages` array in `src/fixtures/scenarios/data.ts` (keep the "deliberately NOT chronological" comment true — just add at the end):

```ts
  {
    id: "fx-msg-11",
    fromInstanceId: AG_DEW,
    toInstanceId: AG_DETORO,
    text: "[task codex-meter-parity] Dew: state — -> review",
    status: "held",
    createdAt: "2026-07-05T12:01:00.000Z",
  },
```

- [ ] **Step 3: Show the badge**

`src/components/ChatRail.tsx:303-305` — directly after the existing `queued` span add:

```tsx
                            {m.status === "held" && (
                              <span className="text-[9px] text-text-tertiary">held</span>
                            )}
```

`src/components/ChatHub.tsx:251-253` — same addition after the `queued` span:

```tsx
                              {m.status === "held" && (
                                <span className="text-[9px] text-text-tertiary">held</span>
                              )}
```

`src/components/ChatHub.tsx:317` — after `{m.status === "queued" && <span className="text-warning">queued</span>}` add:

```tsx
                          {m.status === "held" && <span>held</span>}
```

(Muted, not warning: `held` is the normal in-flight state, not a delivery problem.)

- [ ] **Step 3b: ChatView + StdinBar — widen their local unions, add the `held` branch, pass `immediate: true`** (challenge d157aa7e, Dew; spec ruling 5). These three files are OUTSIDE the task boundary — land them as their own scoped commit (Step 5b).

`src/ipc/commands.ts:255-258`:

```ts
  "message.inject": {
    /** `immediate: true` = the HUMAN's routed send from the composer: delivered
     *  now as a single-item flush, bypassing (and not flushing) the target's
     *  outbox stack. Agents' `tell` and system notifications never set it. */
    req: { fromInstanceId: string; toInstanceId: string; text: string; immediate?: boolean };
    res: InterAgentMessage;
  };
```

`src/components/ChatView.tsx:46` — `status: "queued" | "delivered" | "held";`
`src/components/ChatView.tsx:230-234` — add `immediate: true,` after `text,` in the `ipc.message.inject({ … })` call.
`src/components/ChatView.tsx:413-417` — replace the ternary with:

```tsx
                  {part.status === "delivered" ? (
                    <span>· auto-submit</span>
                  ) : part.status === "held" ? (
                    <span className="text-text-tertiary">· held — delivering in the next batch</span>
                  ) : (
                    <span className="text-warning">· target agent isn't running — queued</span>
                  )}
```

`src/components/StdinBar.tsx:19` — `status: "queued" | "delivered" | "held";`
`src/components/StdinBar.tsx:141-145` — add `immediate: true,` after `text,` in the `ipc.message.inject({ … })` call.
`src/components/StdinBar.tsx:328-332` — same three-branch replacement as ChatView above (same copy, same classes).

Run: `pnpm exec tsc --noEmit` — Expected: clean. (There is no `pnpm typecheck` script in `package.json`; every "pnpm typecheck" in this plan means `pnpm exec tsc --noEmit`.)

- [ ] **Step 5b: Commit the out-of-boundary trio separately**

```bash
git add src/ipc/commands.ts src/components/ChatView.tsx src/components/StdinBar.tsx
git commit -m "feat(chat): routed sends bypass the outbox (immediate) + honest 'held' copy in ChatView/StdinBar" -- src/ipc/commands.ts src/components/ChatView.tsx src/components/StdinBar.tsx
```

- [ ] **Step 4: Pixel gate**

```bash
lsof -nP -iTCP:1420 -sTCP:LISTEN   # kill any dev server from ANOTHER checkout first
pnpm typecheck
pnpm uishot chat
```

Expected: exit 0, `.shots/chat-default.png` written. **Open the PNG with the Read tool** and confirm the Dew → Detoro message shows a small muted `held` label next to it and nothing else moved. Attach the path in the READY note.

- [ ] **Step 5: Commit**

```bash
git add src/ipc/types.ts src/components/ChatRail.tsx src/components/ChatHub.tsx src/fixtures/scenarios/data.ts
git commit -m "feat(chat): render 'held' status for outbox-held inter-agent messages" -- src/ipc/types.ts src/components/ChatRail.tsx src/components/ChatHub.tsx src/fixtures/scenarios/data.ts
```

---

## READY checklist (implementer)

1. All four commits on the lane branch; `git log --oneline main..HEAD` shows exactly them (plus any fixups).
2. Gates recorded on the task: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `pnpm typecheck`, `pnpm uishot chat`.
3. READY note carries: HEAD sha, gate exit codes, the `.shots/chat-default.png` path, and one sentence on any test you had to re-target from `delivered` to `held`.
4. Live behaviour needs an app rebuild + relaunch by the human (dev instance cannot take `conclave.sock`) — say so in the note; do NOT claim a live e2e.
