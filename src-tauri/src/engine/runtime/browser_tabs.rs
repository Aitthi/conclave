//! Pure tab-registry state for the per-agent in-app browser (Lane-1, B1).
//!
//! This module owns the `tabId → tab` map, the active-tab pointer, and the
//! human-tab sequence — with ZERO wry / native-webview dependency, so the whole
//! state machine is unit-testable without a live webview (the native pool in
//! `runtime::browser` mirrors this registry; see the spec's testability seam
//! §5). The serde shapes here ARE the wire contract the frontend consumes
//! (`BrowserState`/`BrowserTab`/`BrowserOwner`, camelCase) — kept byte-identical
//! to the plan's Shared Interface Contract.

use serde::{Deserialize, Serialize};

/// A tab's stable key: the agent id for agent tabs, `"human-<seq>"` for human
/// tabs.
pub type TabId = String;

/// Who owns a tab. Serializes camelCase (`"human"` / `"agent"`) to match the
/// frontend's `OwnerKind` union.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OwnerKind {
    Human,
    Agent,
}

/// Owner descriptor for a tab. `id` is the agent id for agents, or the literal
/// `"human"` for every human tab; `label` is the display name (agent name, or
/// `"You"` for human tabs).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOwner {
    pub kind: OwnerKind,
    pub id: String,
    pub label: String,
}

/// One tab in the registry. `url` is ALWAYS the last-navigated target the
/// caller supplied — NEVER read from the native webview getter (reading
/// `WKWebView.URL` on a fresh `about:blank` child webview panics on the main
/// thread and kills the whole app; see crash history `de0a632f`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTab {
    pub tab_id: TabId,
    pub owner: BrowserOwner,
    pub url: Option<String>,
    pub title: Option<String>,
    pub loading: bool,
    pub ended: bool,
}

/// The full snapshot the `browser.status` command returns: every live tab plus
/// which one is currently visible (`active_tab_id`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserState {
    pub tabs: Vec<BrowserTab>,
    pub active_tab_id: Option<TabId>,
}

/// Viewport rectangle (logical pixels) the Browser tab reserves for the native
/// webview overlay. Mirrored by `BrowserBounds` in `src/ipc/types.ts`.
///
/// It lives HERE rather than in `runtime::browser` because the registry is the
/// single source of truth for the last rect the frontend reported, and decides
/// where a created-or-revealed webview lands. `runtime::browser` re-exports it,
/// so `commands::browser` keeps importing it from there unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Where a freshly created webview must be placed, and whether it may paint at
/// once. `bounds: None` means no rect has ever been reported — the caller parks
/// the webview offscreen until one is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub bounds: Option<Bounds>,
    pub show: bool,
}

/// What activating a tab asks of the native pool: whether the id was known at
/// all, which OTHER webviews to hide first, and the rect to apply to the
/// incoming webview before showing it (inactive webviews hold stale bounds, so
/// a reveal must reposition).
#[derive(Debug, Clone, PartialEq)]
pub struct Activation {
    pub known: bool,
    pub hide: Vec<TabId>,
    pub bounds: Option<Bounds>,
}

/// The owner id stamped on every human tab (the frontend keys human chrome off
/// `owner.kind == "human"`; the id is a constant, the per-tab uniqueness lives
/// in `tab_id`).
const HUMAN_OWNER_ID: &str = "human";
/// The display label for human tabs.
const HUMAN_OWNER_LABEL: &str = "You";

/// Pure, native-free registry of browser tabs: the `tabId → tab` map, the
/// active-tab pointer, and the monotonic human-tab sequence. The native webview
/// pool in `runtime::browser` mirrors this; all tab-lifecycle logic lives here
/// so it can be unit-tested without a live webview.
///
/// Tabs are stored in a `Vec` to preserve insertion order (the side rail renders
/// them top-to-bottom in creation order); N is a handful, so linear lookups are
/// cheaper than a map plus a separate order vector.
#[derive(Debug, Default)]
pub struct TabRegistry {
    tabs: Vec<BrowserTab>,
    active: Option<TabId>,
    human_seq: u64,
    /// Whether the Browser view currently wants the overlay on screen (set by
    /// `set_visible`). A webview created while this is false stays hidden.
    overlay_visible: bool,
    /// The last region rect the frontend reported (set by `set_bounds`), reused
    /// by every later create/reveal so a webview never lands offscreen.
    last_bounds: Option<Bounds>,
}

impl TabRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create-or-reuse the tab keyed by `tab_id`. On create, pushes a fresh tab
    /// (loading, not ended) and — only when there is no active tab yet — makes
    /// it active, so the very first tab is visible without an agent silently
    /// hijacking the human's current view. On reuse, updates the navigation
    /// target and re-enters the loading state. Returns `true` iff a NEW tab was
    /// created.
    pub fn upsert(&mut self, tab_id: TabId, owner: BrowserOwner, url: Option<String>) -> bool {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.tab_id == tab_id) {
            tab.url = url;
            tab.loading = true;
            return false;
        }
        self.tabs.push(BrowserTab {
            tab_id: tab_id.clone(),
            owner,
            url,
            title: None,
            loading: true,
            ended: false,
        });
        if self.active.is_none() {
            self.active = Some(tab_id);
        }
        true
    }

    /// Open a fresh human tab with a monotonic id (`"human-1"`, `"human-2"`, …).
    /// The very first tab becomes active (nothing else is visible yet);
    /// otherwise the human's current active tab is left untouched.
    pub fn new_human_tab(&mut self) -> TabId {
        self.human_seq += 1;
        let tab_id = format!("human-{}", self.human_seq);
        self.tabs.push(BrowserTab {
            tab_id: tab_id.clone(),
            owner: BrowserOwner {
                kind: OwnerKind::Human,
                id: HUMAN_OWNER_ID.to_string(),
                label: HUMAN_OWNER_LABEL.to_string(),
            },
            url: None,
            title: None,
            loading: false,
            ended: false,
        });
        if self.active.is_none() {
            self.active = Some(tab_id.clone());
        }
        tab_id
    }

    /// Make `tab_id` the visible tab. Returns `false` (no-op) if unknown.
    pub fn set_active(&mut self, tab_id: &str) -> bool {
        if self.tabs.iter().any(|t| t.tab_id == tab_id) {
            self.active = Some(tab_id.to_string());
            true
        } else {
            false
        }
    }

    /// Drop the tab keyed by `tab_id`. If it was the active tab, reselect the
    /// first remaining tab (or `None` when the last tab is gone → empty state).
    /// Returns `false` (no-op) if unknown.
    pub fn close(&mut self, tab_id: &str) -> bool {
        let Some(idx) = self.tabs.iter().position(|t| t.tab_id == tab_id) else {
            return false;
        };
        self.tabs.remove(idx);
        if self.active.as_deref() == Some(tab_id) {
            self.active = self.tabs.first().map(|t| t.tab_id.clone());
        }
        true
    }

    /// Mark an agent's tab `ended` (read-only until the human closes it, D4b).
    /// A no-op for a human tab or an unknown id — only agent tabs can end.
    // Live via `runtime::browser::mark_ended`, wired into the terminal
    // agent-exit paths in `instance.rs` (task `inapp-browser-ended-detection`).
    pub fn mark_ended(&mut self, agent_id: &str) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| t.tab_id == agent_id && t.owner.kind == OwnerKind::Agent)
        {
            tab.ended = true;
        }
    }

    /// Clear the ended marker when the same agent identity starts a fresh
    /// runtime generation. A no-op for human tabs and unknown ids.
    pub fn mark_resumed(&mut self, agent_id: &str) {
        if let Some(tab) = self
            .tabs
            .iter_mut()
            .find(|t| t.tab_id == agent_id && t.owner.kind == OwnerKind::Agent)
        {
            tab.ended = false;
        }
    }

    /// Record that a tab finished loading: clears `loading` and stores the page
    /// title. A no-op for an unknown id.
    pub fn set_loaded(&mut self, tab_id: &str, title: Option<String>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.tab_id == tab_id) {
            tab.loading = false;
            tab.title = title;
        }
    }

    /// Record whether the Browser view wants the overlay on screen.
    pub fn set_overlay_visible(&mut self, visible: bool) {
        self.overlay_visible = visible;
    }

    /// Record the region rect the frontend last reported.
    pub fn set_last_bounds(&mut self, bounds: Bounds) {
        self.last_bounds = Some(bounds);
    }

    /// Decide where a webview created for `tab_id` lands and whether it may
    /// paint immediately.
    ///
    /// The caller's rect wins when it has one (it is fresher); otherwise the
    /// last rect the frontend reported, so a webview created between a mount and
    /// the next `set_bounds` still lands over the reserved region instead of
    /// offscreen. It paints only when it IS the active tab AND the overlay is on
    /// screen — the invariant is that exactly one tab is ever visible, and never
    /// while the Browser view is unmounted.
    pub fn placement_for_create(&self, tab_id: &str, requested: Option<Bounds>) -> Placement {
        Placement {
            bounds: requested.or(self.last_bounds),
            show: self.overlay_visible && self.active.as_deref() == Some(tab_id),
        }
    }

    /// Make `tab_id` active and report what the native pool must do about it:
    /// which other webviews to hide first, and the rect to apply to the incoming
    /// one before showing it. An unknown id changes nothing.
    pub fn activate(&mut self, tab_id: &str) -> Activation {
        let known = self.set_active(tab_id);
        let hide = if known {
            self.tabs
                .iter()
                .filter(|t| t.tab_id != tab_id)
                .map(|t| t.tab_id.clone())
                .collect()
        } else {
            Vec::new()
        };
        Activation {
            known,
            hide,
            bounds: self.last_bounds,
        }
    }

    /// The active tab paired with the rect a reveal must apply to it first. A
    /// hidden webview keeps whatever frame it last had, so every reveal path
    /// (`set_visible(true)`, the reselect after a close) repositions before it
    /// shows.
    pub fn active_reveal(&self) -> Option<(TabId, Option<Bounds>)> {
        self.active.clone().map(|id| (id, self.last_bounds))
    }

    /// A read-only snapshot of every tab plus the active pointer.
    pub fn state(&self) -> BrowserState {
        BrowserState {
            tabs: self.tabs.clone(),
            active_tab_id: self.active.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_creates_then_reuses() {
        let mut r = TabRegistry::new();
        let o = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "Guetta".into(),
        };
        assert!(r.upsert("agentA".into(), o.clone(), Some("https://a".into()))); // created
        assert!(!r.upsert("agentA".into(), o, Some("https://b".into()))); // reused
        let s = r.state();
        assert_eq!(s.tabs.len(), 1);
        assert_eq!(s.tabs[0].url.as_deref(), Some("https://b"));
    }

    #[test]
    fn human_tabs_get_sequential_ids() {
        let mut r = TabRegistry::new();
        assert_eq!(r.new_human_tab(), "human-1");
        assert_eq!(r.new_human_tab(), "human-2");
    }

    #[test]
    fn close_active_reselects() {
        let mut r = TabRegistry::new();
        let t1 = r.new_human_tab();
        let t2 = r.new_human_tab();
        r.set_active(&t2);
        assert!(r.close(&t2));
        assert_eq!(r.state().active_tab_id.as_deref(), Some(t1.as_str()));
    }

    #[test]
    fn mark_ended_sets_flag_only_for_agent() {
        let mut r = TabRegistry::new();
        let o = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        r.upsert("agentA".into(), o, None);
        r.mark_ended("agentA");
        assert!(
            r.state()
                .tabs
                .iter()
                .find(|t| t.tab_id == "agentA")
                .unwrap()
                .ended
        );
    }

    #[test]
    fn mark_resumed_clears_ended_for_agent_only() {
        let mut r = TabRegistry::new();
        let owner = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        r.upsert("agentA".into(), owner, None);
        r.mark_ended("agentA");
        r.mark_resumed("agentA");
        assert!(!r.state().tabs[0].ended);
        r.mark_resumed("unknown");
        assert_eq!(r.state().tabs.len(), 1);
    }

    #[test]
    fn first_tab_becomes_active_agents_do_not_hijack() {
        let mut r = TabRegistry::new();
        let agent = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        // First-ever tab (an agent's) becomes active — something must be visible.
        r.upsert("agentA".into(), agent, Some("https://a".into()));
        assert_eq!(r.state().active_tab_id.as_deref(), Some("agentA"));
        // A second agent navigating in the background must NOT steal the view.
        let other = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentB".into(),
            label: "H".into(),
        };
        r.upsert("agentB".into(), other, Some("https://b".into()));
        assert_eq!(r.state().active_tab_id.as_deref(), Some("agentA"));
    }

    #[test]
    fn closing_the_last_tab_clears_active() {
        let mut r = TabRegistry::new();
        let t1 = r.new_human_tab();
        assert!(r.close(&t1));
        let s = r.state();
        assert!(s.tabs.is_empty());
        assert_eq!(s.active_tab_id, None);
    }

    #[test]
    fn set_loaded_clears_loading_and_sets_title() {
        let mut r = TabRegistry::new();
        let o = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        r.upsert("agentA".into(), o, Some("https://a".into()));
        assert!(r.state().tabs[0].loading, "loading while navigating");
        r.set_loaded("agentA", Some("Title".into()));
        let tab = r.state().tabs.into_iter().next().unwrap();
        assert!(!tab.loading);
        assert_eq!(tab.title.as_deref(), Some("Title"));
    }

    // ── First-paint placement (task browser-first-paint) ─────────────────────
    //
    // The human's bug: the FIRST page opened in the in-app browser stays blank
    // until they switch tabs and back. The registry owns the decision the
    // native pool acts on — these tests pin it without a live webview.

    fn rect() -> Bounds {
        Bounds {
            x: 240.0,
            y: 64.0,
            width: 1200.0,
            height: 800.0,
        }
    }

    /// The human flow: New tab → (mounted Browser view has already reported
    /// visibility + a rect) → type a URL → Enter. The webview is created for the
    /// ACTIVE tab while the overlay is on screen, so it must be shown right
    /// away, at the last reported rect — not hidden offscreen awaiting a tab
    /// switch.
    #[test]
    fn navigate_create_shows_when_active_and_overlay_visible() {
        let mut r = TabRegistry::new();
        r.set_overlay_visible(true);
        r.set_last_bounds(rect());
        let t1 = r.new_human_tab();

        let p = r.placement_for_create(&t1, None);
        assert_eq!(
            p,
            Placement {
                bounds: Some(rect()),
                show: true,
            },
            "the active tab's first webview must paint immediately at the reported rect"
        );
    }

    /// The agent-on-empty-rail flow: `browser.open` from an agent makes the
    /// first-ever tab active while the Browser view is mounted — same verdict.
    #[test]
    fn navigate_create_shows_first_agent_tab_on_an_empty_rail() {
        let mut r = TabRegistry::new();
        r.set_overlay_visible(true);
        r.set_last_bounds(rect());
        let owner = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        r.upsert("agentA".into(), owner, Some("https://a".into()));

        let p = r.placement_for_create("agentA", None);
        assert!(p.show, "the first-ever agent tab is active and must paint");
        assert_eq!(p.bounds, Some(rect()));
    }

    /// A background agent opening a SECOND tab must never paint over the tab the
    /// human is on — but it still lands at the reported rect so a later reveal
    /// is a bare `show()`.
    #[test]
    fn navigate_create_hides_when_inactive() {
        let mut r = TabRegistry::new();
        r.set_overlay_visible(true);
        r.set_last_bounds(rect());
        let owner = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        r.upsert("agentA".into(), owner.clone(), None); // first tab → active
        r.upsert("agentB".into(), owner, None); // background

        let p = r.placement_for_create("agentB", None);
        assert_eq!(
            p,
            Placement {
                bounds: Some(rect()),
                show: false,
            },
            "an inactive tab must stay hidden (invariant: only the active tab is visible)"
        );
    }

    /// The overlay is off screen (Browser view unmounted): even the active tab's
    /// fresh webview stays hidden, or it paints over the app chrome.
    #[test]
    fn navigate_create_hides_when_overlay_not_visible() {
        let mut r = TabRegistry::new();
        r.set_last_bounds(rect());
        let t1 = r.new_human_tab();

        // Default is off screen — nothing has mounted the Browser view.
        assert!(!r.placement_for_create(&t1, None).show);
    }

    /// A caller-supplied rect is fresher than the stored one and wins.
    #[test]
    fn placement_prefers_the_callers_rect_over_the_stored_one() {
        let mut r = TabRegistry::new();
        r.set_overlay_visible(true);
        r.set_last_bounds(rect());
        let t1 = r.new_human_tab();

        let fresher = Bounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        assert_eq!(
            r.placement_for_create(&t1, Some(fresher)).bounds,
            Some(fresher)
        );
    }

    /// Inactive webviews hold whatever bounds they were created with, so
    /// activating one must reposition it before showing — otherwise a window
    /// resize while on another tab leaves the incoming page misplaced.
    #[test]
    fn set_active_applies_last_bounds() {
        let mut r = TabRegistry::new();
        let t1 = r.new_human_tab();
        let t2 = r.new_human_tab();
        r.set_last_bounds(rect());

        let act = r.activate(&t2);
        assert!(act.known);
        assert_eq!(act.hide, vec![t1], "every OTHER tab hides first");
        assert_eq!(
            act.bounds,
            Some(rect()),
            "the incoming webview must be repositioned before it is shown"
        );
        assert_eq!(r.state().active_tab_id.as_deref(), Some(t2.as_str()));
    }

    /// An unknown id changes nothing and asks the native pool to touch nothing.
    #[test]
    fn activate_unknown_id_is_a_no_op() {
        let mut r = TabRegistry::new();
        let t1 = r.new_human_tab();
        let act = r.activate("nope");
        assert!(!act.known);
        assert!(act.hide.is_empty());
        assert_eq!(r.state().active_tab_id.as_deref(), Some(t1.as_str()));
    }

    /// Re-showing the overlay (Browser view remounted) must reposition the
    /// active webview first — its bounds are from whenever it was last placed.
    #[test]
    fn set_visible_true_applies_last_bounds() {
        let mut r = TabRegistry::new();
        let t1 = r.new_human_tab();
        r.set_last_bounds(rect());
        r.set_overlay_visible(true);

        assert_eq!(r.active_reveal(), Some((t1, Some(rect()))));
    }

    /// Nothing active → nothing to reveal (graceful no-op, empty rail).
    #[test]
    fn active_reveal_is_none_without_an_active_tab() {
        let r = TabRegistry::new();
        assert_eq!(r.active_reveal(), None);
    }

    /// The wire shape is the frontend contract — pin the camelCase serialization
    /// (`tabId`, `activeTabId`, `"agent"`) so a stray field rename can't silently
    /// break Lane-2's `BrowserState`/`BrowserTab` types.
    #[test]
    fn state_serializes_camelcase_to_the_interface_contract() {
        let mut r = TabRegistry::new();
        let o = BrowserOwner {
            kind: OwnerKind::Agent,
            id: "agentA".into(),
            label: "G".into(),
        };
        r.upsert("agentA".into(), o, Some("https://a".into()));
        let v = serde_json::to_value(r.state()).unwrap();
        assert_eq!(v["activeTabId"], "agentA");
        let tab = &v["tabs"][0];
        assert_eq!(tab["tabId"], "agentA");
        assert_eq!(tab["owner"]["kind"], "agent");
        assert_eq!(tab["loading"], true);
        assert_eq!(tab["ended"], false);
    }
}
