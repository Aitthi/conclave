# Right-Rail Chats Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The right panel becomes **Chats** — a read-only, real-time viewer of agent conversations (room switcher + live stream + read-only footer) — replacing the Context drawer. The drawer's Context content (Skills, Session, Memory·Snapshots, meter) moves into the **center pane** as two slim click-to-open popover bars.

**Design record:** blackboard key `design:right-rail-chat` (Arta, human-directed) + prototype `.arta/proto/screens/chats.tsx`, `.arta/proto/components/AppShell.tsx`, `.arta/proto/lib/data.ts`, `.arta/proto/theme.css`. **The proto `.tsx` files are canonical**; the PNG snapshots under `.arta/snapshots/` include stale iterations (a composer in the rail, Chat|Context tabs) that do NOT ship.

**Architecture:** Frontend-only. No new backend — the rail reuses the Chat Hub's data layer (`ipc.message.listForWorkspace`, `useAnyMessageInjected`, `derivePairs`, `ClampText`). A shared hook is extracted from `ChatHub.tsx`; a new `ChatRail` replaces `ContextDrawer` in `WorkspacePane.tsx`; the drawer's live sections are extracted (not rewritten) into two slim center-pane bars; `ContextDrawer.tsx` is then deleted. The center **Chat Hub stays** — the rail's maximize button opens it (`onOpenChat`).

**Tech Stack:** React + TypeScript + Tailwind, lucide-react icons.

## Reconciliation rulings (design ↔ backend reality)

Recorded by the lead (Detoro) after design review; these bind every task:

- **R1 — Rooms are derived, not modeled.** `InterAgentMessage` is pairwise (`fromInstanceId`/`toInstanceId`); there is no channel/group/thread entity. Phase 1 rooms = **`#workspace`** (the all-messages feed) **+ one DM room per pair** from `derivePairs`. The proto's named group threads ("uds-socket-steal") and the "+ Start a thread" button are **out of scope** (need a backend thread entity — recorded follow-up).
- **R2 — No typing indicator.** There is no honest typing signal. The proto's typing bubble is dropped. Live dots map to real instance status (`running`), same basis as the tab-strip dots.
- **R3 — Unread badges are client-side.** Last-seen per room key, in-memory for the app session (workspace-scoped state, same discipline as the drawer's collapse state). No fake persistence.
- **R4 — Context meter** joins the bottom bar as a compact chip (design gap in proto; default ruled by lead, Arta may restyle later).
- **R5 — Non-CLI agents.** The drawer's chat-agent (Model·API) and orchestrator (Fusion panel/judge/cost) sections fold into a **Config popover** on the same top bar, shown only for those agent types. The rail itself is workspace-scoped and identical for all types.
- **R6 — Tools placeholder** ("coming in M5") is dropped; it returns when M5 lands.
- **R7 — The rail has no filter/search in Phase 1** — search lives in the Chat Hub; the rail's header keeps only maximize ("Open in Chat Hub") and collapse.
- **R8 — Agents only; the human does not appear (HUMAN DIRECTIVE, 2026-07-04).** The rail shows inter-agent traffic exclusively: no "You" bubbles, no human DM rooms, no human in the member counts. This matches data reality — the human's direct messages go via `ipc.message.send` straight into a session and are never rows in `InterAgentMessage`; only agent↔agent injections are. (Honesty note: a human-ROUTED injection sent from agent A's composer is recorded as `fromInstanceId = A` and is indistinguishable from A's own message — it renders as A, which is accepted.) The proto's `you` participant, `you-arta`/`you-mellow` DM rooms, and right-aligned `msg-mine` bubbles do NOT ship.

## Global Constraints

- All UI copy is **English** (workspace rule — replies to the human are Thai, the app is not).
- **Honesty rule:** render only data that genuinely exists; fetch errors get a visible inline note, never a silent empty state.
- **Extract, don't rewrite:** the drawer's Session/Snapshots/meter logic carries hard-won guards (StrictMode spawn trap, `mounted` ref + monotonic `seq` fetch guards, restart lockout timers). Move the code and its comments; do not re-derive it.
- **Terminal invariant:** every cli tab's xterm stays always-mounted with visibility toggling (`WorkspacePane.tsx` comment at the terminal layer). No new conditional may wrap or remount those layers.
- **Auto-scroll:** port the Chat Hub's near-bottom-guarded auto-scroll (`NEAR_BOTTOM_PX`) — a background refetch must never scroll-jack a reader.
- Frontend gate before every commit: `npx tsc --noEmit` from the repo root, green.
- Commits end with `Co-Authored-By:` trailer per repo convention.
- Theme: real tokens from `src/styles/app.css` — the proto's `theme.css` mirrors them; when they disagree, `app.css` wins.

---

### Task 1: Extract the shared workspace-chat hook

**Files:**
- Create: `src/lib/useWorkspaceChat.ts`
- Modify: `src/components/ChatHub.tsx`

**Interfaces:**
- Produces: `useWorkspaceChat(workspaceId: string): { messages: InterAgentMessage[]; loadError: boolean; identityOf: (id: string) => AgentIdentity; refetch: () => void }` plus the exported `AgentIdentity` type and `FALLBACK_IDENTITY`. Tasks 2–3 consume it.
- Source: lift the identities join (`ipc.instance.list` ⨝ `ipc.agentDef.list`), the seq-guarded `listForWorkspace` refetch, and the `useAnyMessageInjected(() => refetch())` subscription **verbatim** from `ChatHub.tsx` (lines ~51–113).

- [ ] Step 1: Create the hook by moving the code (keep `MESSAGE_LIMIT = 200` inside the hook; keep comments).
- [ ] Step 2: Refactor `ChatHub` onto the hook — behavior-preserving, no visual change.
- [ ] Step 3: Verify `npx tsc --noEmit`; manual smoke: open Chat Hub, messages render, live injection still refreshes.
- [ ] Step 4: Commit.

### Task 2: Room derivation

**Files:**
- Create: `src/lib/chatRooms.ts`

**Interfaces:**
- Consumes: `InterAgentMessage[]` (newest-first, as the API returns), `derivePairs`/`pairKeyOf` from `src/lib/chatPairs.ts`, a `lastSeen: Record<string, string>` map (room key → last-seen message id or ISO time).
- Produces: `deriveRooms(messages, lastSeen): Room[]` where `Room = { key: string; kind: "channel" | "dm"; title: string; memberIds: string[]; lastAt: string; unread: number }`. Room `key`s: literal `"workspace"` for the channel; the `pairKeyOf` key for DMs. `#workspace` is always first and always present (even with zero messages); pairs follow most-recent-first. `unread` counts messages newer than `lastSeen[key]` (0 when unseen entry is absent → everything unread is wrong for first open; rule: absent entry = fully read at mount, so unread only accrues while the app runs — no fake history badges).
- Pure function, no React.

- [ ] Step 1: If the repo has a frontend test runner (check `package.json` for vitest/jest), write unit tests for: workspace-room always present, pair grouping, unread accrual, absent-lastSeen = read. If none exists, note it in the commit body and rely on Step 2.
- [ ] Step 2: Verify `npx tsc --noEmit`.
- [ ] Step 3: Commit.

### Task 3: `ChatRail` — the read-only right rail

**Files:**
- Create: `src/components/ChatRail.tsx`

**Interfaces:**
- Props: `{ workspaceId: string; roster: RoutingTarget[]; statuses: Record<string, WorkspaceAgent["status"]>; onOpenChat?: () => void }`.
- Consumes: `useWorkspaceChat` (Task 1), `deriveRooms` (Task 2), `ClampText`, `timeHint`.
- Layout per proto `chats.tsx`, adapted by rulings: header row ("Chats" + total unread + maximize → `onOpenChat` + collapse), horizontal room-chip switcher (avatar, title, unread badge, live dot per R2), thin active-room meta line (members · N live), message stream (Today divider, consecutive same-sender grouping like the proto's `group()`; every message left-aligned — agents only per R8, there is no "mine" side; DM rooms render exactly like the channel, just filtered — Slack-style, do NOT port the Chat Hub's two-sided `pairKeyOf` pair view into the rail), near-bottom-guarded auto-scroll, and the read-only footer: *"Read-only live view — agents reply from their own terminal."*
- Collapse: same pattern as the drawer's current Show/Hide (a slim vertical strip when collapsed); state is workspace-scoped in-memory.
- Room selection + `lastSeen` live in component state; selecting a room marks it seen; the active room accrues seen continuously while scrolled to bottom.

- [ ] Step 1: Build the component (not yet mounted).
- [ ] Step 2: Verify `npx tsc --noEmit`.
- [ ] Step 3: Commit.

### Task 4: Mount the rail — `WorkspacePane` swap

**Files:**
- Modify: `src/components/WorkspacePane.tsx`

**Interfaces:**
- Replace the `<ContextDrawer …/>` mount (currently inside `activeTab !== null`) with `<ChatRail workspaceId={workspaceId} roster={roster} statuses={…from tabs}} onOpenChat={onOpenChat} />` mounted **unconditionally** (the rail is workspace-scoped, not per-tab — it must not unmount on tab switch or when no tab is active).
- `ContextDrawer` import stays until Task 7 (the file still compiles); only the mount moves.

- [ ] Step 1: Swap the mount; thread `statuses` from `tabs`.
- [ ] Step 2: Verify `npx tsc --noEmit`; manual smoke: rail shows live messages, tab switching does not reset the rail, maximize opens the Chat Hub, collapse works.
- [ ] Step 3: Commit.

### Task 5: Center top bar — Skills popover + Session actions + Config popover

**Files:**
- Create: `src/components/ContextBars.tsx` (exports `ContextTopBar`, `ContextBottomBar`)
- Modify: `src/components/WorkspacePane.tsx`

**Interfaces:**
- `ContextTopBar` props: `{ def: AgentDefinition; status: WorkspaceAgent["status"]; instanceId: string; session: Session | null; launchedSkillIds?: string[] }` — the same data the drawer received.
- Content per proto `AppShell.tsx` top section: `Skills (N) ▾` trigger → popover listing the effective skill set **with the existing drift hint** ("restart to apply", from `launchedSkillIds` diff) and the Role chip (ADR 0005 section); divider; Resume-last-handoff + Restart icon buttons wired to the drawer's existing Session logic (move the lockout/timeout code as-is). For `chat`/`orchestrator` defs add the `Config ▾` popover (R5) holding the drawer's Model·API / Fusion sections.
- Mount: in `WorkspacePane`'s `<main>`, directly under the tab strip, rendered from `activeTab` data (one instance, active agent only). Popovers close on outside click and are mutually exclusive with the bottom bar's (proto behavior).

- [ ] Step 1: Extract the Skills/Role/Session/Config code from `ContextDrawer.tsx` into `ContextTopBar` (move guards + comments).
- [ ] Step 2: Mount under the tab strip.
- [ ] Step 3: Verify `npx tsc --noEmit`; manual smoke: skills list + drift hint, restart/resume still work, popover open/close.
- [ ] Step 4: Commit.

### Task 6: Center bottom bar — Snapshots popover + handoff summary + meter chip

**Files:**
- Modify: `src/components/ContextBars.tsx`, `src/components/WorkspacePane.tsx`

**Interfaces:**
- `ContextBottomBar` props: same shape as top bar.
- Content per proto bottom section + R4: `Snapshots (N) ▾` trigger → popover with the drawer's full snapshot list (expand row → saved content, delete, submit-into-terminal, "Snapshot now", "Compact now") — move the logic verbatim; inline summary text `last handoff ~N tok · age`; Compact icon button; compact context-meter chip (R4) fed by the drawer's existing `session:context` estimate code, honest-labelled.
- Mount: bottom of the active agent's column, visually **above the composer** per proto (top bar / terminal / bottom bar / StdinBar). Exact DOM placement relative to the per-tab always-mounted layers is implementer judgment under two constraints: the terminal invariant (no remounting), and at most ONE live snapshot/meter fetcher at a time (don't duplicate per hidden tab).

- [ ] Step 1: Extract snapshots + meter code into `ContextBottomBar`.
- [ ] Step 2: Mount per the constraint above.
- [ ] Step 3: Verify `npx tsc --noEmit`; manual smoke: snapshot list/expand/delete/submit, compact fires, meter updates, resume gating (Session's Resume enable follows snapshot presence — that wiring crosses both bars; keep the shared state in `WorkspacePane` or a small context, not duplicated fetches).
- [ ] Step 4: Commit.

### Task 7: Delete `ContextDrawer` + sweep

**Files:**
- Delete: `src/components/ContextDrawer.tsx`
- Modify: any remaining importers (`WorkspacePane.tsx`)

**Interfaces:** none new.

- [ ] Step 1: Remove the import + file. `grep -rn "ContextDrawer" src/` must return nothing.
- [ ] Step 2: The drawer's Messages policy header (Accepts-from / Auto-submit) dies with it — confirm that policy info still has a home: it does, in the Builder/agent config; note in commit body.
- [ ] Step 3: Verify `npx tsc --noEmit`; full manual smoke of the three panes.
- [ ] Step 4: Commit.

## Risk ledger

- **`ContextDrawer.tsx` is 997 lines of guarded code** — the restart lockout timer, StrictMode-safe spawn results, seq-guarded fetches. Rewriting any of it from memory WILL reintroduce fixed bugs. Move code blocks wholesale.
- **Terminal always-mounted invariant** (`WorkspacePane.tsx` terminal-layer comment): scrollback and TUI mouse modes die if a bar mount restructures those layers. Test wheel-scroll after tab switching in Task 6.
- **Rail must be workspace-scoped**: mounting it per-tab (like the drawer was) resets room selection and lastSeen on every tab switch — that's the bug the unconditional mount in Task 4 exists to prevent.
- **`Resume` gating crosses bars**: the drawer computed "has snapshots" once and gated Session's Resume with it. Split into two bars, the state must be lifted, not fetched twice.
- **No human sender-detection needed (R8)**: do not port `ChatView`'s user/assistant sides into the rail — the message table has no human identity, and inventing one would violate the honesty rule.
- **ChatHub refactor (Task 1) must be behavior-preserving** — it shipped reviewed at `c5465d1`; any visible change there is a defect.
