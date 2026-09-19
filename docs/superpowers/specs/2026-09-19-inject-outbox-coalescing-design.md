# Inject outbox: coalesce inter-agent messages into one turn

**Status:** approved by the human 2026-09-19 (Detoro brainstorm, four rulings below).
**Owner:** Detoro (lead) · **authority:** in-loop.

## Problem

Every inter-agent message reaches a target agent through
`commands::message::inject` (`src-tauri/src/engine/commands/message.rs`), and
`inject` delivers each call on its own: one bracketed paste into the target's
PTY, then three spaced `\r` keystrokes. A Claude Code receiver therefore sees
one *user turn per message*.

The standard handoff protocol produces three such messages within seconds,
from three different sources, all landing on the same target:

1. `conclave task note <ws> <slug> "READY …"` → `notify_watchers` (system)
2. `conclave task state <ws> <slug> review` → `notify_watchers` (system)
3. `conclave tell <owner> "…"` written by the agent (agent)

The receiver processes three turns, re-reads the same task three times, and
pays three rounds of output tokens for one piece of news. Screenshots from
2026-09-19 07:55 and 08:30 show exactly this stack on Detoro's and Mellow's
panes.

## Goal

Hold messages destined for the same target in a short-lived per-target
**outbox**, then deliver the whole stack as **one paste + one submit** so the
receiver handles them in a single turn. Human input is never held.

## Rulings (human, 2026-09-19)

| # | Question | Ruling |
|---|----------|--------|
| 1 | What happens to a held stack when the human types to the same agent? | **Leave the stack alone.** Human input (`message.send`) bypasses the outbox entirely and does not flush it. |
| 2 | Maximum hold time? | **None.** Bounded by item count instead. |
| 3 | Item cap | **10 items.** Reaching 10 flushes immediately; the 11th starts a new stack. |
| 4 | Timer rule | See *Timer* below. Same constants for system and agent sources. |
| 5 (amendment, Detoro ruling on challenge d157aa7e by Dew, confirmed by Mellow, 2026-09-19) | The human's **routed send** (composer → "send to agent X" in `StdinBar.tsx` / `ChatView.tsx`) does not use `message.send`; it calls `message.inject` with the human's own agent as sender. Ruling 1 says human input is never held, so `message.inject` gains an `immediate: bool` flag (default `false`). The two UI routed-send call sites pass `immediate: true`; the engine then delivers that one message now, as a single-item flush, and **leaves the target's pending stack untouched**. CLI `tell` and every system caller never set it. |

## Timer

Per target stack, with `INITIAL_WAIT = 10s`, `RESET_WINDOW = 3s`,
`EXTEND = 5s`, `MAX_ITEMS = 10`:

| Event | Effect |
|-------|--------|
| First item enters an empty stack | `deadline = now + INITIAL_WAIT` |
| Next item arrives **≤ RESET_WINDOW** after the previous item | `deadline = now + INITIAL_WAIT` (reset) |
| Next item arrives **> RESET_WINDOW** after the previous item | `deadline = deadline + EXTEND` (remaining 1s → 6s) |
| Stack reaches `MAX_ITEMS` | flush now, inside the caller; next item starts a fresh stack |
| `now ≥ deadline` (sweeper) | flush |

"Previous item" means the arrival time of the last pushed item, not the
stack's first item. Worked example from the 07:55 screenshot: READY note at
t=0 (deadline 10), state→review at t=1 (≤3s, reset → 11), agent `tell` at t=9
(8s gap > 3s, remaining 2s + 5 → 14). All three deliver at t≈14 in one turn.

No `source` field is introduced: the human set identical constants for system
and agent traffic, so a discriminator would be dead weight (YAGNI). The four
constants live in the outbox module so a later per-source table is a local
change.

## Architecture

### New module `src-tauri/src/engine/runtime/outbox.rs`

```rust
pub struct Outbox { stacks: Mutex<HashMap<String /* to_instance_id */, Stack>> }

struct Stack { items: Vec<HeldItem>, deadline: Instant, last_arrival: Instant }

pub struct HeldItem {
    pub row_id: String,           // inter_agent_message.id, already persisted as "held"
    pub from_instance_id: String,
    pub sender_name: String,
    pub text: String,             // RAW text; the [from …] tag is applied at flush
}

pub enum Push { Held, Flush(Vec<HeldItem>) }

impl Outbox {
    pub fn push(&self, to: &str, item: HeldItem, now: Instant) -> Push;
    /// Drain every stack whose deadline has passed. Pure over `now`.
    pub fn take_due(&self, now: Instant) -> Vec<(String, Vec<HeldItem>)>;
    #[cfg(test)] pub fn take_all(&self, to: &str) -> Vec<HeldItem>;
}
```

`push` and `take_due` are synchronous and hold the `Mutex` only for the map
mutation — no I/O under the lock. The sweeper (`outbox::run(state)`) ticks
every 250 ms, calls `take_due`, and delivers each drained stack via
`message::flush_stack`. Same spawn idiom as `task_timer::run` in `lib.rs`.
Worst-case added latency beyond the deadline is one tick.

### `commands::message`

- `inject` accepts an optional `immediate: bool` (serde default `false`) and
  keeps its validation, lifecycle-lock ordering and eligibility re-check
  exactly as today, then:
  1. persists the row with status **`"held"`** (new status value);
  2. builds a `HeldItem`; if `immediate` is `true`, releases its guards and
     calls `flush_stack(state, &to, vec![item])` directly — the target's
     pending stack is neither flushed nor touched (ruling 5); otherwise calls
     `outbox.push`;
  3. on `Push::Flush(items)` releases its guards and calls
     `flush_stack(state, &to, items).await` before returning;
  4. returns the row (status `held`, or `delivered`/`queued` when a cap flush
     or an immediate delivery settled it synchronously — `flush_stack`
     returns the final status).
- New `flush_stack(state, to, items)`:
  1. take the target's agent lifecycle lock (owned guard, same helper
     `inject` uses);
  2. re-check `require_delivery_eligible(to)`; if it fails, mark every row
     `queued` and return;
  3. body = items mapped to `[from {sender_name} · {from_instance_id}] {text}`
     joined by a blank line (`"\n\n"`), FIFO;
  4. `runtime.send_stdin_paste(to, &body)` then the existing
     `SUBMIT_CR_DELAYS_MS` loop — this block moves out of `inject` verbatim;
  5. `Ok` → update every row to `delivered`, then emit `bus::MESSAGE_INJECTED`
     **once per item** (unchanged payload shape) so the UI keeps rendering
     one bubble per message; `NotLive` → rows `queued`, no event; `Closed` →
     rows `queued`, log at warn (a flush has no caller to return an error to).
- `SUBMIT_CR_DELAYS_MS` and the paste-envelope reasoning stay where they are;
  only the caller changes.

### `repo::inter_agent_message`

- `update_status(db, ids: &[String], status: &str)` — one `UPDATE … WHERE id IN (…)`.
- `requeue_held(db)` — `UPDATE inter_agent_message SET status='queued' WHERE status='held'`;
  called once at engine start (in `lib.rs`, before the sweeper spawns). Stacks
  are in-memory only; an app restart turns any held row into `queued`, the same
  status a message to an offline target already gets today (deliver-on-spawn
  remains the open TODO it is now).

### Callers

Unchanged call sites — `notify_watchers`, `notify_expected_ruler`
(`commands/task.rs`), `task_timer.rs`, CLI `tell` (`commands/cli.rs`) — keep
calling `message::inject` with the same payload. They observe a `held` ack
instead of `delivered`; none of them read the status.

`message.send` (StdinBar / Terminal pane, own agent) is **not touched**
(ruling 1). The human's routed send (StdinBar / ChatView → another agent)
uses `message.inject` and passes `immediate: true` (ruling 5).

### UI / TS

- `src/ipc/types.ts`: `status: "queued" | "delivered" | "held"`.
- `src/ipc/commands.ts` `message.inject` req: `immediate?: boolean`.
- `ChatRail.tsx`, `ChatHub.tsx` (two sites) currently show a `queued` badge;
  show a `held` badge the same way (text `held`, muted color).
- `ChatView.tsx` and `StdinBar.tsx` each declare their OWN narrowed
  `"queued" | "delivered"` union and render `delivered ? "· auto-submit" :
  "· target agent isn't running — queued"`. Widen both unions to include
  `"held"` and add a third, muted branch: `· held — delivering in the next
  batch`. Their routed-send calls pass `immediate: true`, so on the happy
  path they still see `delivered`; the `held` branch is the honest rendering
  if the ack ever is `held` (amended from "needs no change" — challenge
  d157aa7e, Dew; verified by Mellow).
- Fixture scenarios: add one `held` message so `pnpm uishot chat` exercises
  the badge.

## Error handling

| Case | Behaviour |
|------|-----------|
| Target stopped between `held` and flush | rows → `queued`, no PTY write, no event |
| Backend channel closed at flush | rows → `queued`, `tracing::warn!` |
| Engine restart with held rows | `requeue_held` at start → `queued` |
| Sender/target invalid | `inject` still errors before persisting (unchanged) |
| Self-injection (from == to) | goes through the stack like any other message |

## Testing

Unit (pure, `outbox.rs`):
- first push sets `deadline = now + 10s`;
- push at +1s resets to `now + 10s`; push at +4s (gap > 3s) extends by 5s;
- 10th push returns `Flush` with all 10 items FIFO; 11th push starts a new stack;
- `take_due` drains only stacks with `deadline ≤ now`, leaves others intact.

Integration (`commands/message.rs` tests, `LiveHandle::for_test_pty`):
- rewrite `inject_live_target_retries_submit_cr`: inject twice within the
  window, call `flush_stack` with `take_all`, assert **one** bracketed paste
  containing both tagged lines (in order, blank-line separated) followed by
  `SUBMIT_CR_DELAYS_MS.len()` bare `\r`;
- inject returns status `held` and the row is `held` in the DB;
- flush against a `NotLive` target marks rows `queued` and emits nothing;
- cap: 10 injects deliver synchronously (row 10 reads `delivered`).

Repo:
- `requeue_held` flips only `held` rows.

Gates (recorded via `conclave task gate`): `cargo test`, `cargo clippy
--all-targets -- -D warnings`, `cargo fmt --check`, `pnpm typecheck`,
`pnpm uishot chat` (pixel gate: open the PNG).

## Out of scope

- Deliver-on-spawn drain of `queued` rows (existing TODO).
- Per-source timer constants (add a `source` field only if the values diverge).
- Merging human input with a pending stack (ruling 1).
- Any change to `message.send`, the PTY paste envelope, or `SUBMIT_CR_DELAYS_MS`.
