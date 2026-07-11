# In-App Browser Multi-Tab (Per-Agent) + Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: this plan is executed as **Conclave lanes** — one task object per lane with its own `--boundary`, delegated to peer agents in isolated worktrees. Steps use checkbox (`- [ ]`) syntax. Within a lane, follow superpowers:test-driven-development for the TDD-marked steps.

**Goal:** Turn the singleton in-app browser into per-agent tabs (each agent/session gets its own live native webview), give the human own-tabs + a read-only view of every agent tab, and redesign the view as a vertical owner-labelled side rail.

**Architecture:** The backend converts the one shared wry child-webview into a `tabId → webview` registry (N concurrent webviews; only the active one visible, the rest hidden-but-alive). Owner identity for agent tabs is derived server-side from the authenticated caller. The frontend polls a new `BrowserState` (list of tabs) and renders a side rail. The redesign is authored in the **Conclave Design view (design-host)** — NOT `.arta/` — and the frontend follows it, gated by the UI Pixel Gate.

**Tech Stack:** Rust (Tauri/wry) backend; React + TypeScript frontend; Conclave design-host for the mockup; `pnpm uishot` for pixel verification; `cargo test -p conclave` for Rust.

**Spec:** `docs/superpowers/specs/2026-07-11-inapp-browser-multitab-redesign-design.md` (commit `c43acdb`)

## Global Constraints

Every task inherits ALL of these:

- **UI copy is English** (not Thai) — this is app UI. (Replies to the human stay Thai.)
- **NEVER read the native webview URL getter.** Use the caller's navigation target (`state_with_url()` pattern, `runtime/browser.rs:427-444`). Reading `WKWebView.URL` on a fresh `about:blank` child webview panics on the main thread and kills the whole app.
- **eval results must be JSON-safe** before crossing the native bridge; every injected script stays a try/catch IIFE returning `{__error}` rather than throwing (`runtime/browser.rs:14-21`).
- **Owner id is derived server-side from the authenticated caller** — NEVER trust a client-supplied owner (anti-spoof).
- **Rust builds/tests run from `src-tauri/`**: `cd src-tauri && cargo test -p conclave <filter>` (repo-root cargo fails exit 101 — no Cargo.toml there).
- **UI Pixel Gate** (CLAUDE.md, blackboard `protocol:ui-pixel-gate`): any `src/` UI change → `pnpm uishot <view>` for each affected view (+ `--scenario empty` when it affects empty states), **open and Read each PNG**, attach shot paths in the READY note, and record `conclave task gate <ws> <slug> -- pnpm uishot <view>`.
- **Fixtures use fixed literal timestamps** (no `Date.now()`); a missing fixture handler THROWS by design — add the handler, never swallow.
- **Tauri getters can throw *synchronously* in plain Chrome** — wrap the getter call itself in try/catch (reference `src/lib/fileDrop.ts`), never rely on a promise `.catch()`.
- **Fresh lane worktrees have no `node_modules`** — run `pnpm install` once before any `pnpm`/`tauri` gate.
- **Shared checkout:** commit only your boundary paths (`conclave stage commit`, or `git commit -- <paths>`), never a bare `git commit` (sweeps peers' staged work).
- **Design canon:** Lane-2 does not improvise the visual — it follows Lane-0's Conclave Design-view canon; visual disputes escalate to Arta.
- **GUARD (boundary — caller-scoped commands):** any new agent-scoped CLI/engine command is threaded through `src-tauri/src/bin/conclave-cli.rs` (`expand_self_args`/`CONCLAVE_INSTANCE_ID`), `src-tauri/src/engine/commands/cli.rs` (argv→method map), and `src-tauri/src/engine/router.rs` (dispatch arm) — NOT only the command's own module. A boundary that names only the feature module (e.g. `commands/browser.rs`) cannot deliver caller identity. Include all three whenever caller-id or a new method is involved. (Root cause of challenge a281ebf9.)
- **GUARD (boundary — TS type renames):** renaming an exported IPC type ripples into the barrel `src/ipc/index.ts` re-export; include it in any lane boundary that renames `src/ipc/types.ts` exports.
- **GUARD (boundary — design lane = `design/**`):** a Conclave Design-view lane should take the whole `design/**` project, not a screen-scoped subset. The Design view renders ONE screen per `design/screens/<id>.tsx` (id = filename), so distinct app states are distinct screen files; and the `conclave design review` gate asserts A2_shared (a component imported by ≥2 screens — satisfy with a shared `TabRail`). Since the designer is the sole owner of `design/` and it never overlaps `src/`/`src-tauri/`, always boundary a design lane as `design/**`. (Root cause of challenge bf0dfa82.)
- **GUARD (design gate — do NOT add `design/config.json`):** in the current design-host build `Shell.tsx` has no react-router `Router` and no `hashchange` listener (screens switch via Switcher React state only). Adding `design/config.json` flips gate assertion **A3 from skip → FAIL** (0 reachable nav targets; a cross-screen `<Link>` crashes the canvas with "destructure basename of useContext null"). Omit it — A3 skips and the gate stays green; the frontend never reads it. Enabling A3 is a separate design-host infra task (add a Router/hashchange), not a design-lane deliverable. (Accepted deviation on task inapp-browser-design-canon, verified live by Arta.)

---

## Shared Interface Contract (fixed by the lead — do not renegotiate privately)

Both Lane-1 (Rust, serde) and Lane-2 (TS) build to THIS exact shape. Field names and types are identical across the boundary.

```ts
// src/ipc/types.ts  (Rust: serde-serialized, camelCase via rename_all)
type OwnerKind = "human" | "agent";

interface BrowserOwner {
  kind: OwnerKind;
  id: string;        // agentId for agents; "human" for human tabs
  label: string;     // display name: agent name, or "You" for human tabs
}

interface BrowserTab {
  tabId: string;     // agentId for agent tabs; "human-<seq>" for human tabs
  owner: BrowserOwner;
  url?: string;      // last-navigated target — NEVER read from the native getter
  title?: string;
  loading: boolean;
  ended: boolean;    // agent finished; tab is read-only until human closes it
}

interface BrowserState {
  tabs: BrowserTab[];
  activeTabId?: string;
}
```

Command signatures (facade `src/ipc/commands.ts`, backend `commands/browser.rs`):

| Command | Args | Returns | Caller | Notes |
|---------|------|---------|--------|-------|
| `browser.status` | — | `BrowserState` | frontend + rail | replaces old single-page status |
| `browser.newTab` | — | `{ tabId }` | frontend (human "+") | creates a human tab `human-<seq>` |
| `browser.goto` | `{ tabId, url }` | `BrowserState` | frontend (human URL bar) | navigate a specific **human** tab |
| `browser.setActive` | `{ tabId }` | `BrowserState` | frontend (row click) | switch the visible webview |
| `browser.setBounds` | `{ rect }` | — | frontend | active-tab overlay rect (unchanged) |
| `browser.setVisible` | `{ visible }` | — | frontend | show/hide whole overlay (unchanged) |
| `browser.close` | `{ tabId }` | `BrowserState` | frontend | close one tab |
| `browser.open`/`goto`/`eval`/`click`/`type`/`snapshot`/`screenshot` | (existing) | (existing) | **agents via CLI** | auto-scoped to `tabId = callerAgentId` server-side |

---

## File Structure

**Lane-1 (backend, Rust):**
- Create: `src-tauri/src/engine/runtime/browser_tabs.rs` — pure `TabRegistry` (state map, no wry). Unit-testable.
- Modify: `src-tauri/src/engine/runtime/browser.rs` — replace singleton with registry-driven multi-webview; thin native wrapper.
- Modify: `src-tauri/src/engine/commands/browser.rs` — owner derivation from caller; verb→registry mapping; `BrowserState` return; new `newTab`/`setActive`/`close(tabId)`/`goto(tabId,url)`.
- Modify: `src-tauri/src/bin/conclave-cli.rs`, `src-tauri/src/engine/commands/cli.rs`, `src-tauri/src/engine/router.rs` — thread the caller's agent id into browser verbs via the `expand_self_args`/`CONCLAVE_INSTANCE_ID` idiom (the browser CLI arm at `commands/cli.rs:784` currently passes NO caller id; agent-scoped verbs must inject it like `tell`/`task claim`). **Added post-plan** per Tiësto's challenge a281ebf9 — caller-id auto-scoping is unreachable from the original 3-file boundary.

**Lane-2 (frontend, TS/React):**
- Modify: `src/ipc/types.ts:413-418` — replace `BrowserStatus` with `BrowserTab`/`BrowserState`/`BrowserOwner`.
- Modify: `src/ipc/commands.ts:349-359,489-501` — new `browser.*` facade signatures.
- Modify: `src/components/InAppBrowserView.tsx` (249) — tab-list state, side rail, human/agent chrome, active overlay.
- Modify: `src/fixtures/scenarios/default.ts:84-98` and `empty.ts:38-45` — return `BrowserState`.
- Modify (if needed): `src/components/AppShell.tsx:203-218` — rail badge from tab count.

**Lane-0 (design, Conclave Design view):**
- Create: `design/screens/browser.tsx` (repo root; filename = screen id `browser`; `export const meta = { title }` pure literal + default-export component). In-repo, committed.
- Create as needed: `design/components/*.tsx` (side rail, rows), `design/lib/*` (state). Tokens already in `design/theme.css`.
- Setup (Step 0, one-time): the workspace is auto-registered into `~/.conclave/design-host/registry.json` (id→dir map pointing at the repo's `design/`) the first time the **human opens the Design view** — that registry does not exist yet for codeup, so the human must open the Design view once before Arta can serve the screen. No filesystem manifest to edit (`listScreens` auto-discovers `design/screens/*.tsx`).

---

## Lane-0 — Design canon (owner: Arta, tool: Conclave Design view)

> Delegated to Arta (designer), who owns the Conclave Design view. This lane authors the visual in-repo under `design/` (NOT `.arta/`), so its boundary (`design/**`) never overlaps Lane-1 (`src-tauri/**`) or Lane-2 (`src/**`). **Blocks Lane-2's visual acceptance** — Lane-2 may build structure/fixtures against this plan before the canon lands, but its UI Pixel Gate is judged against this canon and Arta's sign-off.

**Boundary:** `design/**` (the whole in-repo design project: `design/screens/browser.tsx` + `design/screens/browser-empty.tsx` + `design/components/**` + `design/lib/**` + `design/theme.css` + `design/config.json`). Arta is the sole owner of the workspace design project; `design/**` has ZERO overlap with Lane-1 (`src-tauri/**`) or Lane-2 (`src/**`), so the broad boundary carries no collision risk. (Widened from the single `browser.tsx` → `browser*.tsx` → `design/**` across two rounds of lead rulings on challenge bf0dfa82: the design-review gate's **A2_shared** assertion needs a component imported by ≥2 screens — satisfied by `browser.tsx` + `browser-empty.tsx` both importing `TabRail` — and **A3** requires `design/config.json`; both are unreachable from a screen-only boundary. Reproduced by Arta running `conclave design review codeup`.)

**Deliverable — `design/screens/browser.tsx` covering every state:**
- Side rail (vertical, Arc-like): a row per tab = owner avatar/name + page title + status dot.
- Tab states: active, inactive, **loading**, **agent "ended"** (read-only badge).
- Chrome variants: **human tab** (URL bar + refresh enabled) vs **agent tab** (URL bar read-only/locked, no nav controls).
- **Empty state** (no tabs).
- Consistency with the center-pane mount contract (floating blurred header; `MemoryGraph.tsx:~445`) where it composes with the new side rail.

**Steps:**
- [ ] **Step 0 (human, one-time):** human opens the Conclave Design view once so the engine's `design.ensure` upserts codeup into `~/.conclave/design-host/registry.json` (pointing at the repo's `design/`). The Design view has never been opened for codeup — without this the host cannot serve the screen.
- [ ] Arta authors `design/screens/browser.tsx` (+ `design/components/*` rows/rail as needed), iterating with the human live in the Design view. Tokens from `design/theme.css`.
- [ ] Arta runs `conclave design review codeup` → **zero SERIOUS findings** + Arta's blind rubric pass.
- [ ] Human approves the visual direction in the Design view.
- [ ] **Gate/deliverable:** Arta commits the finished screen, records the **pinned SHA on blackboard key `design:inapp-browser`**, which becomes Lane-2's task `--canon`. Arta is the escalation target for Lane-2 visual disputes and runs the **design-acceptance gate** (reviews Lane-2's `pnpm uishot browser` PNGs against this canon before merge).

---

## Lane-1 — Backend multiplex (owner: a Rust implementer; highest risk)

**Boundary:** `src-tauri/src/engine/runtime/browser.rs`, `src-tauri/src/engine/runtime/browser_tabs.rs`, `src-tauri/src/engine/commands/browser.rs`, `src-tauri/src/bin/conclave-cli.rs`, `src-tauri/src/engine/commands/cli.rs`, `src-tauri/src/engine/router.rs`. (Last 3 added by lead ruling on challenge a281ebf9 — caller-id threading lives there; verified by Tiësto + Mellow + lead, zero overlap with other lanes.)

**Resolve FIRST (risk #2):** how existing agent-scoped engine calls obtain the caller's agent id (UDS session / engine router context — see memory *Engine UDS JSON-RPC direct*). Owner derivation depends on it. Post findings as a task note before Task B3.

### Task B1: Pure `TabRegistry` (state map — full TDD)

**Files:**
- Create: `src-tauri/src/engine/runtime/browser_tabs.rs`
- Test: same file `#[cfg(test)] mod tests`

**Interfaces — Produces:**
```rust
pub type TabId = String;
#[derive(Clone, serde::Serialize)] #[serde(rename_all = "camelCase")]
pub enum OwnerKind { Human, Agent }
#[derive(Clone, serde::Serialize)] #[serde(rename_all = "camelCase")]
pub struct BrowserOwner { pub kind: OwnerKind, pub id: String, pub label: String }
#[derive(Clone, serde::Serialize)] #[serde(rename_all = "camelCase")]
pub struct BrowserTab { pub tab_id: TabId, pub owner: BrowserOwner,
    pub url: Option<String>, pub title: Option<String>, pub loading: bool, pub ended: bool }
#[derive(Clone, serde::Serialize)] #[serde(rename_all = "camelCase")]
pub struct BrowserState { pub tabs: Vec<BrowserTab>, pub active_tab_id: Option<TabId> }

pub struct TabRegistry { /* map + active + human seq */ }
impl TabRegistry {
    pub fn new() -> Self;
    /// create-or-reuse; sets url/loading; returns whether a NEW tab was created
    pub fn upsert(&mut self, tab_id: TabId, owner: BrowserOwner, url: Option<String>) -> bool;
    pub fn new_human_tab(&mut self) -> TabId;          // "human-<seq>"
    pub fn set_active(&mut self, tab_id: &str) -> bool; // false if unknown
    pub fn close(&mut self, tab_id: &str) -> bool;      // reselect active if it was active
    pub fn mark_ended(&mut self, agent_id: &str);       // owner.kind==Agent
    pub fn set_loaded(&mut self, tab_id: &str, title: Option<String>);
    pub fn state(&self) -> BrowserState;
}
```

- [ ] **Step 1 (TDD): failing test — upsert creates then reuses**
```rust
#[test]
fn upsert_creates_then_reuses() {
    let mut r = TabRegistry::new();
    let o = BrowserOwner { kind: OwnerKind::Agent, id: "agentA".into(), label: "Guetta".into() };
    assert!(r.upsert("agentA".into(), o.clone(), Some("https://a".into())));   // created
    assert!(!r.upsert("agentA".into(), o, Some("https://b".into())));          // reused
    let s = r.state();
    assert_eq!(s.tabs.len(), 1);
    assert_eq!(s.tabs[0].url.as_deref(), Some("https://b"));
}
```
- [ ] **Step 2:** `cd src-tauri && cargo test -p conclave upsert_creates_then_reuses` → FAIL (no such type/fn).
- [ ] **Step 3:** implement the struct + `upsert` minimally to pass.
- [ ] **Step 4:** rerun → PASS.
- [ ] **Step 5 (TDD): failing tests — human seq, set_active reselect, close, mark_ended**
```rust
#[test]
fn human_tabs_get_sequential_ids() {
    let mut r = TabRegistry::new();
    assert_eq!(r.new_human_tab(), "human-1");
    assert_eq!(r.new_human_tab(), "human-2");
}
#[test]
fn close_active_reselects() {
    let mut r = TabRegistry::new();
    let t1 = r.new_human_tab(); let t2 = r.new_human_tab();
    r.set_active(&t2);
    assert!(r.close(&t2));
    assert_eq!(r.state().active_tab_id.as_deref(), Some(t1.as_str()));
}
#[test]
fn mark_ended_sets_flag_only_for_agent() {
    let mut r = TabRegistry::new();
    let o = BrowserOwner { kind: OwnerKind::Agent, id: "agentA".into(), label: "G".into() };
    r.upsert("agentA".into(), o, None);
    r.mark_ended("agentA");
    assert!(r.state().tabs.iter().find(|t| t.tab_id=="agentA").unwrap().ended);
}
```
- [ ] **Step 6:** run the three → FAIL.
- [ ] **Step 7:** implement `new_human_tab`/`set_active`/`close`/`mark_ended`/`set_loaded`/`state` to pass.
- [ ] **Step 8:** `cd src-tauri && cargo test -p conclave browser_tabs` → all PASS.
- [ ] **Step 9:** commit (`conclave stage commit`, boundary = browser_tabs.rs).

### Task B2: Native webview wrapper keyed by tabId (interface + invariants + manual gate)

> Native wry integration resists line-by-line TDD (the crash history proves behavior surfaces only against a live webview). Implement to the interface + invariants below; verify with the live gate, not fabricated unit code.

**Files:** Modify `src-tauri/src/engine/runtime/browser.rs`.

**Interfaces — Consumes:** `TabRegistry` (B1). **Produces:** an internal `WebviewPool` the command layer drives:
- `ensure(tab_id) -> ()` — `add_child(WebviewBuilder(label = format!("{BROWSER_LABEL}:{tab_id}"), …))` if absent; else no-op.
- `navigate(tab_id, url)`, `show_only(tab_id)` (`setVisible(false)` all others, `true` + bounds on target), `set_bounds(rect)` (active only), `destroy(tab_id)`.

**Invariants (from Global Constraints — enforce, do not restate as code you can't verify):**
- Never read the native URL; the registry holds `url` from the navigation target.
- `show_only` sets others hidden BEFORE showing the target (no wrong-page flash).
- eval path unchanged (JSON-safe IIFE), now per-label.

- [ ] Replace the singleton `BROWSER_LABEL` webview with the pool; keep the existing hide-keeps-alive semantics.
- [ ] Wire `TabRegistry` as the source of truth; native pool mirrors it.
- [ ] **Gate (live, manual):** run the app; drive two tabs; confirm switching shows the right page and neither read-URL nor eval crashes. Record as a task note with steps. (No `.ips` = check `~/Library/Application Support/Conclave/panic.log` on any death.)
- [ ] Commit (boundary = browser.rs).

### Task B3: Command layer — owner derivation + BrowserState (partial TDD)

**Files:** Modify `src-tauri/src/engine/commands/browser.rs`.

**Interfaces — Consumes:** `TabRegistry`, `WebviewPool`, the caller-id mechanism (from the FIRST-resolve note). **Produces:** the `browser.*` commands per the Interface Contract table.

- [ ] **Step 1 (TDD): failing test — agent verb scopes to caller id**
  Test the pure mapping `resolve_owner(caller) -> (tabId, BrowserOwner)` with a stub caller context: an agent caller `agentA/"Guetta"` yields `tabId="agentA"`, `owner.kind=Agent`; a client-supplied `owner` field is ignored.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** implement `resolve_owner` + wire `open/goto/eval/...` through it into the registry/pool.
- [ ] **Step 4:** run → PASS.
- [ ] **Step 5:** implement `status`→`BrowserState`, `newTab`, `setActive`, `close(tabId)`, `goto(tabId,url)`; agent-end hook calls `registry.mark_ended`.
- [ ] **Step 6:** `cd src-tauri && cargo test -p conclave browser` → PASS; `cargo build` green.
- [ ] **Step 7:** commit (boundary = commands/browser.rs).

### Task B4: CLI/agent-end wiring + live multi-agent gate

- [ ] Confirm `conclave browser <verb>` reaches the engine carrying caller identity (no CLI change if the engine derives it; otherwise thread it). Note findings.
- [ ] ~~Hook agent teardown → `mark_ended(agentId)`~~ — **DEFERRED to follow-up task `inapp-browser-ended-detection`** (ruling on Tiësto's D4b escalation, verified by Mellow). The genuine crash-death signal is `commands/instance.rs forward_session_output()` (~978-1245): a detached, epoch-guarded, shared-by-every-agent EOF forwarder with a documented prior ordering race — hooking it is a fragile concurrency change, unrelated to the browser boundary, that must not be rushed into this lane. The `ended` flag + `mark_ended` method (B1) + the frontend's ended chrome (Lane-2, fixture-exercised) all SHIP now; only the call-site that flips the flag is deferred. Tab **persistence** (no auto-close) ships regardless — the human closes ended agent tabs manually until the follow-up lands.
- [ ] **Gate (live):** two agents each `conclave browser open <different url>`; assert `browser.status` shows two agent tabs, non-colliding, each independently navigable. (The ended-badge is exercised via the Lane-2 fixture, not the live agent-exit path — see the deferral above.) Record gate note.

---

## Lane-2 — Frontend tab-aware view + side rail (owner: a frontend implementer)

**Boundary:** `src/ipc/types.ts`, `src/ipc/commands.ts`, `src/ipc/index.ts` (barrel re-export of the renamed types), `src/components/InAppBrowserView.tsx`, `src/fixtures/scenarios/default.ts`, `src/fixtures/scenarios/empty.ts`, `src/components/AppShell.tsx` (rail badge / browserActive fallout). **Consumes:** the Interface Contract + Lane-0 canon. Can build on fixtures before Lane-1 lands; integrates at merge.

### Task F1: IPC types + facade signatures (TDD via tsc)

**Files:** Modify `src/ipc/types.ts:413-418`, `src/ipc/commands.ts:349-359,489-501`.
- [ ] Replace `BrowserStatus` with `BrowserOwner`/`BrowserTab`/`BrowserState` (Interface Contract, verbatim field names).
- [ ] Update the `browser.*` facade to the new signatures (`status→BrowserState`, add `newTab`/`goto(tabId,url)`/`setActive`/`close(tabId)`).
- [ ] `pnpm tsc --noEmit` → expect errors only in `InAppBrowserView.tsx`/fixtures (updated next). Commit types+facade (boundary paths).

### Task F2: Fixtures return BrowserState (TDD via uishot readiness)

**Files:** Modify `src/fixtures/scenarios/default.ts:84-98`, `empty.ts:38-45`.
- [ ] `default.ts` `browser.status` → `BrowserState` with 1 human + 2 agent tabs (one `ended:true`), `activeTabId` = the human tab; add `newTab`/`goto`/`setActive`/`close` handlers returning updated state. **Fixed literal timestamps.**
- [ ] `empty.ts` `browser.status` → `{ tabs: [], activeTabId: undefined }`.
- [ ] Commit (boundary = fixtures).

### Task F3: InAppBrowserView rewrite + side rail (UI Pixel Gate)

**Files:** Modify `src/components/InAppBrowserView.tsx`.
- [ ] Replace scalar state with tab-list state from `browser.status()` polling (keep the 2s `reconcile`).
- [ ] Render the **side rail** per Lane-0 canon: owner avatar/name + title + status; row click → `setActive`; "+" → `newTab`.
- [ ] Human tab: URL bar + refresh enabled (`goto(tabId,url)`). Agent tab: URL bar disabled/read-only; "ended" badge when `ended`.
- [ ] Active tab's native webview overlaid on `regionRef` (unchanged mechanism); empty state when no tabs.
- [ ] **UI Pixel Gate:** `pnpm install` (fresh worktree) → kill any foreign :1420 server (`lsof -nP -iTCP:1420 -sTCP:LISTEN`) → `pnpm uishot browser` and `pnpm uishot browser --scenario empty` → **open and Read both PNGs** → confirm against Lane-0 canon → attach paths in READY note → `conclave task gate <ws> <slug> -- pnpm uishot browser`.
- [ ] `pnpm tsc --noEmit` green. Commit (boundary = InAppBrowserView.tsx).

### Task F4: AppShell rail badge (optional, only if canon calls for a count)

- [ ] If the design shows a tab-count badge, wire it from `browser.status().tabs.length` at `AppShell.tsx:203-218`. Else skip. Commit if changed.

---

## Integration (owner: Detoro, lead)

Merge order: **Lane-0 canon → Lane-1 backend → Lane-2 frontend.** Frontend may develop on fixtures in parallel but its UI Pixel Gate is judged against the landed canon, and real-webview behavior is verified only after Lane-1 merges. Lead re-runs every gate at integration and attributes any red to a lane. `conclave lane finish` per lane after merge.

**Post-integration live proof (lead):** rebuild+relaunch (human), then two real agents each open a browser tab → confirm non-collision + human read-only view + ended-badge, per Lane-1 B4 gate.

---

## Self-Review

- **Spec coverage:** D1 per-agent (B1/B2 registry+pool), D2 human/agent chrome (F3), D3 side rail (Lane-0+F3), D4a agent=1 tab/human seq (B1 upsert/new_human_tab), D4b ended-persist (B1 mark_ended + F3 badge), CLI auto-scope (B3 resolve_owner), BrowserState (Interface Contract, B1+F1), redesign→Conclave Design view `design/screens/browser.tsx` (Lane-0), risk#2 caller-id (Lane-1 FIRST-resolve), UI Pixel Gate (F3). All covered.
- **Placeholder scan:** no TBDs — Lane-0 authoring path (`design/screens/browser.tsx`), pin (`design:inapp-browser` SHA), and gate (`conclave design review codeup` + Arta rubric) are all concrete (confirmed with Arta, the tool owner).
- **Type consistency:** `BrowserState`/`BrowserTab`/`BrowserOwner` field names identical in the Interface Contract, Rust (B1, camelCase serde), and TS (F1). `mark_ended`, `set_active`, `new_human_tab`, `resolve_owner` used consistently.
