# In-App Browser — Multi-Tab (Per-Agent) + Redesign

**Date:** 2026-07-11
**Author:** Detoro (lead), with the human (requester)
**Status:** Approved design — ready for implementation planning
**Supersedes/extends:** the current single-webview in-app browser (`InAppBrowserView.tsx`, `runtime/browser.rs`)

---

## 1. Goal (requester's words)

> "ทำ inapp browser ให้ Support หลาย tab หน่อย และ Design หน้า inapp browser ใหม่ด้วย"

Make the in-app browser support **multiple tabs**, and **redesign** the in-app browser view.

The concrete problem being solved: today the in-app browser is a **singleton** native webview shared by every agent and the human. When two agents drive the browser at once (via `conclave browser …`), they collide on the one webview. Multi-tab makes each agent's browsing an **independent surface**, and gives the human their own browsing tab plus a read-only view of every agent's tab.

## 2. Settled decisions (from the brainstorm)

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Per-agent surfaces.** Tabs are keyed by owner; each agent/session gets its own webview, keyed by owner id. The human can view every tab. | The real pain is agents colliding on one shared webview. Human-only Chrome-style tabs would not fix that. |
| D2 | **Human has own tab(s) with full control; agent tabs are read-only** for the human (view only, URL bar locked, no navigation). | Don't let the human hijack an agent mid-task. Safe, unambiguous. |
| D3 | **Vertical side rail (Arc-like) layout.** Tabs stacked on the left, each showing owner avatar/name + page title + status. | Surfaces "who owns this tab" — the defining element of per-agent tabs — and scales as agents come and go. |
| D4a | **Tab count per owner:** human may open **multiple** manual tabs; **each agent gets exactly one** tab, reused on re-navigation (v1). | Keeps the `conclave browser` CLI contract simple ("my tab"). YAGNI on multi-tab-per-agent. |
| D4b | **Agent-tab lifecycle:** when an agent finishes/dies, its tab **persists as read-only with an "ended" badge** until the human closes it. | Lets the human review what an agent browsed after the fact; nothing vanishes. |

## 3. Rejected alternatives

- **iframe-based tabs (trivial in React DOM).** ❌ The in-app browser is driven by agents through `conclave browser` (eval/click/snapshot). A cross-origin iframe cannot be scripted, which breaks agent driving. Native webview is required, not a choice.
- **One native webview, swap URL on tab switch.** ❌ Switching would reload the page (losing scroll/JS state) and, fatally, an agent's tab could not keep running in the background while another tab is active. This fails D1's "agents don't collide / run concurrently."
- **Human can take over / drive an agent's tab.** ❌ Rejected at D2 — risks disrupting an agent mid-task.
- **Auto-close an agent's tab when it finishes.** ❌ Rejected at D4b — the human loses the ability to review.
- **LRU eviction of idle webviews.** Deferred to v2 (see §9 risk ledger). v1 keeps all live tabs; note the memory cost.

## 4. Architecture

### 4.1 Ownership & tab identity

- **`tabId`** is the unique key for a tab and the suffix of the native webview label: `BROWSER_LABEL:<tabId>` (today the label is the bare singleton `BROWSER_LABEL`, `runtime/browser.rs:513-514`).
  - **Agent tab:** `tabId == <agentId>`. Exactly one per agent (D4a). Reused on re-navigation.
  - **Human tab:** `tabId == "human-<seq>"` (a monotonic per-session sequence). Multiple allowed (D4a).
- Each tab carries an **owner descriptor**: `{ kind: "human" | "agent", id, label }` where `label` is the display name (agent name, or "You" for human tabs).
- **Active tab:** the backend tracks exactly one `activeTabId`. Only that tab's webview is **visible**; every other webview is `setVisible(false)` but **stays alive and loaded** (matches the existing "X hides the page but keeps it running for background agents" behavior — `InAppBrowserView.tsx:168`).

### 4.2 Backend multiplex (the hard, risky part — `runtime/browser.rs`, 1004 lines)

Convert the singleton webview into a **registry of webviews keyed by `tabId`** (N concurrent native child webviews).

- **Create:** on first navigation for a `tabId`, `add_child` a new `WebviewBuilder(label = BROWSER_LABEL:<tabId>, …)`. On subsequent navigations to an existing `tabId`, reuse it (`goto`).
- **Switch active:** `setActive(tabId)` → `setVisible(false)` on the outgoing active webview, `setVisible(true)` + apply current bounds on the incoming one. Must be atomic-ish so the wrong page never flashes.
- **Bounds:** `setBounds(rect)` applies to the **active** webview only. Inactive webviews keep their last bounds; they are hidden anyway.
- **Close:** `close(tabId)` tears down that one webview and drops it from the registry. If it was active, pick another tab as active (or none → empty state).
- **Ended agent tabs (D4b):** when an agent ends, mark its tab `ended: true` in the registry but **do not** tear down the webview until the human closes it. It renders read-only with an "ended" badge.

**Non-negotiable safety rules carried forward (multiply per webview):**
- **NEVER read the native webview URL getter.** `state_with_url()` returns the caller's navigation target instead (`runtime/browser.rs:427-444`). Reading `WKWebView.URL` on a fresh `about:blank` child webview panics on the main thread and kills the whole app (task `browser-crash-fix`, commits `82b134a`/`376c916`; memory `de0a632f`).
- **eval results must be forced JSON-safe** before the native bridge or `NSJSONSerialization` aborts the app without hitting the panic hook (task `browser-eval-json-safety`, `86b9cfa`/`822143e`; memory `d41c0e06`). Every injected script stays a try/catch IIFE returning `{__error}` rather than throwing (`runtime/browser.rs:14-21`).
- **eval on a tab still at initial `about:blank`** returns "eval callback was dropped" (wry discards the callback until a real load drains `pending_scripts`) — unchanged, applies per tab.

### 4.3 `conclave browser` CLI contract change (affects every agent that browses)

Today every verb (`open/goto/status/snapshot/click/type/eval/close/setBounds/setVisible/screenshot`, declared `commands.ts:349-359`) targets the one singleton webview. New contract:

- The agent-driving verbs **auto-scope to the calling agent's own tab** (`tabId = <callerAgentId>`). The agent does not pass a tab id; the engine derives owner identity **server-side from the authenticated caller** (do NOT trust a client-supplied owner — see §9).
- Transparent to agents: they simply get their own private surface instead of a shared one.
- `browser.status` (see §4.4) returns the **full tab list**, not a single page.

### 4.4 IPC surface (`src/ipc/`)

- `BrowserStatus` (today `{ ok, url?, title?, message? }`, `src/ipc/types.ts:413-418`) is replaced by a **tab list** shape:
  ```ts
  interface BrowserTab {
    tabId: string;
    owner: { kind: "human" | "agent"; id: string; label: string };
    url?: string;         // last-navigated target (never read from native getter)
    title?: string;
    loading: boolean;
    ended: boolean;       // agent finished; read-only
  }
  interface BrowserState { tabs: BrowserTab[]; activeTabId?: string }
  ```
- Commands the **frontend** uses (facade `commands.ts:489-501`):
  - `browser.status()` → `BrowserState` (all tabs).
  - `browser.newTab()` → creates a human tab (`tabId = human-<seq>`), returns its `tabId`.
  - `browser.goto(tabId, url)` → navigate a specific **human** tab (the URL bar path).
  - `browser.setActive(tabId)` → switch the visible tab.
  - `browser.setBounds(rect)` → active-tab overlay rect (unchanged mechanism, `InAppBrowserView.tsx:38-40`).
  - `browser.setVisible(bool)` → show/hide the whole browser overlay (unchanged).
  - `browser.close(tabId)` → close one tab.
- **Events:** none added — keep the existing **polling** model (`reconcile()` every 2s, `InAppBrowserView.tsx:62-74`; Rail poll every 4s, `AppShell.tsx:203-218`). An event bus is out of scope.

### 4.5 Frontend view (`InAppBrowserView.tsx`, 249 lines → tab-aware)

- Replace scalar single-page state with **tab-list state** driven by `browser.status()` polling.
- **Left side rail** (per D3): one row per tab = owner avatar/name + title + loading/ended status; a **"+" button** creates a human tab. Clicking a row → `setActive(tabId)`.
- **Human tab:** URL bar + refresh enabled (`goto(tabId, url)`).
- **Agent tab:** URL bar **disabled/read-only**, shows the agent's current URL + an "ended" badge when `ended`. No navigation controls. (Read-only is UI-enforced in v1; see §9.)
- The active tab's native webview is overlaid on the measured region exactly as today (`regionRef`, `InAppBrowserView.tsx:233`).
- Follow the **center-pane mount contract** where it fits (floating blurred header, absolute-inset-0; template `MemoryGraph.tsx:~445`, memory `a861b8e4`) — Arta reconciles the new side rail with this shell in the canon.

### 4.6 Redesign → Conclave Design view (design canon)

- The redesign is authored in the **Conclave Design view (design-host)** — **NOT** `.arta/`. For this workspace the design canvas is **in-repo**: screens live at `design/screens/*.tsx` at the repo root (committed; `welcome.tsx` today). `~/.conclave/design-host/registry.json` is only an id→dir map pointing back at the repo's `design/` (`design-host/vite/projects.ts`, `screens.ts listScreens`).
- **Arta produces `design/screens/browser.tsx`** (filename = screen id `browser`; `export const meta={title}` + default-export component), with side-rail rows/chrome in `design/components/*` and tokens from `design/theme.css`. It covers all states (active/inactive/loading/ended, human vs agent chrome, empty). This is the **design canon** the frontend lane pins (committed SHA on blackboard `design:inapp-browser`) and must not improvise around.
- **One-time setup:** the Design view has never been opened for codeup, so `~/.conclave/design-host/registry.json` has no codeup entry yet. The human must open the Design view once (fires `design.ensure`) before Arta can serve the screen. `listScreens` auto-discovers `design/screens/*.tsx` — no manifest to edit.
- **Lane-0 gate:** `conclave design review codeup` → zero serious findings + Arta's blind rubric pass.
- The frontend lane satisfies the **UI Pixel Gate** (CLAUDE.md): `pnpm uishot browser` (+ `--scenario empty`), open and Read each PNG, attach paths in the READY note, record the gate. Arta reviews those PNGs against the canon (design-acceptance) before merge.

## 5. Component boundaries (for isolation)

| Unit | Purpose | Depends on |
|------|---------|-----------|
| **Webview registry** (`runtime/browser.rs`) | Owns the `tabId → webview` map, active-tab tracking, create/reuse/switch/close/ended. | wry `add_child`, `setVisible`, `setBounds` |
| **Browser command layer** (`commands/browser.rs`) | Maps IPC/CLI verbs to registry ops; **derives owner id from the authenticated caller**; returns `BrowserState`. | registry |
| **IPC facade** (`src/ipc/commands.ts`, `types.ts`) | Typed `browser.*` seam + `BrowserState`/`BrowserTab` types. | — |
| **View** (`InAppBrowserView.tsx`) | Side-rail UI, tab switching, human vs agent chrome, overlay bounds. | IPC facade, fixtures |
| **Design canon** (`design/screens/browser.tsx`, Conclave Design view) | The pinned visual spec for the view. | — (Arta) |
| **Fixtures** (`src/fixtures/scenarios/{default,empty}.ts`) | Fixture `browser.status` returning a tab list so uishot renders the chrome without Tauri. | types |

Testability seam: keep the **pure tab-registry logic** (state map: create/reuse/switch/close/ended keying) separable from the native webview calls, so the registry can be unit-tested without a live webview (native/bundle webview paths are not exercisable from `cargo test` — split pure logic from the native wrapper, per the resolver-splitting precedent in memory). Native calls stay a thin wrapper the registry drives.

## 6. Data flow

1. **Agent browses:** `conclave browser open <url>` → engine resolves caller agentId → registry create/reuse `tabId=agentId` → webview navigates. Agent's verbs (`eval/click/…`) all scope to that tab.
2. **Human browses:** side rail "+" → `browser.newTab()` → `tabId=human-<seq>` → URL bar `goto(tabId,url)`.
3. **Human views an agent tab:** click the agent's row → `setActive(agentTabId)` → its (already-alive) webview becomes visible; URL bar is read-only.
4. **Poll:** view calls `browser.status()` every 2s → repaints the tab list + active overlay.
5. **Agent ends:** engine marks `ended:true`; tab stays read-only until human `close(tabId)`.

## 7. Error handling & edge cases

- **Fixture / plain-Chrome mode:** no Tauri, so the webview region is empty by design (`InAppBrowserView.tsx:11-12`); the side rail + chrome must still render from the fixture tab list. Any `@tauri-apps/api` getter on the render path can throw **synchronously** — wrap the getter itself in try/catch (reference `src/lib/fileDrop.ts`), never rely on `.catch()`.
- **Fresh tab at `about:blank`:** never read its native URL; `state_with_url()` uses the navigation target. eval before first real load returns "callback dropped" — expected.
- **Closing the active tab:** pick a remaining tab as active, or fall to the empty state if none.
- **Duplicate navigation for an agent:** reuse the existing `tabId=agentId` webview (D4a), do not spawn a second.
- **Owner spoofing:** owner id is derived server-side from the authenticated caller; a client-supplied owner is ignored (§9).

## 8. Testing

- **Backend (Rust):** unit-test the pure tab-registry (create/reuse/switch/close/ended, active-tab selection) without a live webview. Integration test the command layer's owner-derivation (caller id → tabId) with a stub registry.
- **Frontend:** update `src/fixtures/scenarios/default.ts:84-98` → `browser.status` returns a multi-tab `BrowserState` (1 human + 2 agents, one `ended`); `empty.ts:38-45` → `{ tabs: [], activeTabId: undefined }`. Run the **UI Pixel Gate**: `pnpm uishot browser` and `pnpm uishot browser --scenario empty`, Read both PNGs, attach paths.
- **Type/lint:** `pnpm tsc --noEmit`. **Rust:** `cd src-tauri && cargo test -p conclave <filters>` (cargo lives in `src-tauri/`; running from repo root fails with exit 101 — memory).

## 9. Risk ledger (known-fragile)

1. **Native crash paths multiply per webview.** Every rule from `browser-crash-fix` / `browser-eval-json-safety` now applies N times. Never read the native URL getter; keep eval JSON-safe. Highest-risk area of the whole change.
2. **Caller-identity threading (backend).** The command layer must resolve the caller's agent id server-side to key the tab. The implementer must find how existing agent-scoped engine calls obtain caller identity (UDS session / engine router context — see memory `Engine UDS JSON-RPC direct`) and reuse it. **Do not** accept an owner id from the client payload (spoofing). This is the load-bearing unknown — resolve it first.
3. **Memory: N webviews = N WebKit processes.** v1 keeps all live tabs (no eviction). Note the cost; if it bites, LRU eviction of idle *human* tabs (never live agent tabs) is the v2 lever. Log, don't silently cap.
4. **Active-switch flash.** `setVisible(false)` old before `setVisible(true)` new + bounds, or the wrong page flashes during a switch.
5. **Read-only is UI-enforced in v1.** The frontend simply omits navigation controls for agent tabs. Backend-level rejection of human-initiated navigation on an agent tab is deferred (agents can only reach their own tab anyway via server-side keying).
6. **Polling latency.** Tab list refreshes on the 2s poll; a just-created agent tab appears up to 2s late. Acceptable (no event bus in scope).

## 10. Scope & phasing (lanes)

- **Lane-0 (design, Arta — Conclave Design view):** `design/screens/browser.tsx` canon — side-rail mockup, all tab states + empty state. Boundary `design/**`. **Blocks Lane-2.**
- **Lane-1 (backend, Rust — highest risk):** webview registry multiplex + command layer owner-keying + `BrowserState`. Parallel to Lane-0. Resolve risk #2 first.
- **Lane-2 (frontend):** tab-aware `InAppBrowserView` + side rail per canon + IPC types + fixtures + UI Pixel Gate. Starts after Lane-0 canon lands; consumes Lane-1's `BrowserState` shape (agree the TS/Rust type at the boundary up front).
- **Integration:** lead (Detoro) owns the merge order (canon → backend → frontend) and re-runs gates.

**Explicitly out of scope (v1):** multiple tabs per agent; human take-over of agent tabs; event-bus push (keep polling); LRU eviction; backend-enforced read-only. **D4b auto ended-badge (the `mark_ended` call-site on agent process-exit)** is deferred to a follow-up task `inapp-browser-ended-detection` — the real crash-death signal lives in the fragile, shared `commands/instance.rs forward_session_output()` EOF forwarder (documented race), which must not be rushed into the browser lane. The `ended` flag, `mark_ended` registry method, and frontend ended-chrome all ship in v1 (fixture-exercised); tab **persistence** ships (no auto-close), so the human closes ended agent tabs manually until the follow-up wires the flag.

## 11. Pointers

- Frontend view: `src/components/InAppBrowserView.tsx` (249) · mount `src/components/AppShell.tsx:527-533`, state `:67`, `:150`, poll `:203-218`.
- Backend: `src-tauri/src/engine/runtime/browser.rs` (1004; label/add_child `:513-514`, `state_with_url` `:427-444`, IIFE `:14-21`) · `src-tauri/src/engine/commands/browser.rs` (242).
- IPC: `src/ipc/commands.ts:349-359` (surface) / `:489-501` (facade) · `src/ipc/types.ts:413-418` (`BrowserStatus`).
- Fixtures: `src/fixtures/scenarios/default.ts:84-98`, `empty.ts:38-45`.
- Design canon: `design/screens/browser.tsx` via Conclave Design view (design-host: `design-host/vite/projects.ts`, `screens.ts`; registry `~/.conclave/design-host/registry.json` → repo `design/`); pin on blackboard `design:inapp-browser`; gate `conclave design review codeup`. Mount contract: `MemoryGraph.tsx:~445` (memory `a861b8e4`). UI Pixel Gate: `CLAUDE.md`, blackboard `protocol:ui-pixel-gate`.
- Crash history: memory `de0a632f` (about:blank URL panic), `d41c0e06` (eval JSON safety).
