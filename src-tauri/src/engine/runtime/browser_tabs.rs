//! Pure tab-registry state for the per-agent in-app browser (Lane-1, B1).
//!
//! This module owns the `tabId → tab` map, the active-tab pointer, and the
//! human-tab sequence — with ZERO wry / native-webview dependency, so the whole
//! state machine is unit-testable without a live webview (the native pool in
//! `runtime::browser` mirrors this registry; see the spec's testability seam
//! §5). The serde shapes here ARE the wire contract the frontend consumes
//! (`BrowserState`/`BrowserTab`/`BrowserOwner`, camelCase) — kept byte-identical
//! to the plan's Shared Interface Contract.

use serde::Serialize;

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

    /// Record that a tab finished loading: clears `loading` and stores the page
    /// title. A no-op for an unknown id.
    pub fn set_loaded(&mut self, tab_id: &str, title: Option<String>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.tab_id == tab_id) {
            tab.loading = false;
            tab.title = title;
        }
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
