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
    /// when there is none). Test-only: the hot path drains through
    /// [`Outbox::take_due`] or a cap flush, never by target id.
    #[cfg(test)]
    pub fn take_all(&self, to: &str) -> Vec<HeldItem> {
        let mut stacks = self.stacks.lock().unwrap_or_else(|e| e.into_inner());
        stacks.remove(to).map(|s| s.items).unwrap_or_default()
    }

    /// Current deadline of `to`'s stack, if any. Test-only accessor — the
    /// sweeper compares deadlines inside [`Outbox::take_due`].
    #[cfg(test)]
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
        assert_eq!(
            deadline_of(&ob, "A"),
            t0 + Duration::from_secs(4) + INITIAL_WAIT
        );
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
        assert_eq!(
            deadline_of(&ob, "A"),
            t1 + INITIAL_WAIT,
            "11th item starts a new stack"
        );
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
        assert!(
            ob.take_due(t0 + INITIAL_WAIT).is_empty(),
            "nothing else due"
        );
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
        assert_eq!(
            Outbox::tagged_line(&items[0]),
            "[from Sender1 · from-1] msg 1"
        );
        assert_eq!(
            Outbox::body(&items),
            "[from Sender1 · from-1] msg 1\n\n[from Sender2 · from-2] msg 2"
        );
    }
}
