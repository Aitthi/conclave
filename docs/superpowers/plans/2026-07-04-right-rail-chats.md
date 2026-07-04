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
- Produces: `deriveRooms(messages, lastSeen): Room[]` where `Room = { key: string; kind: "channel" | "dm"; title: string; memberIds: string[]; lastAt: string; unread: number }`. Room `key`s: literal `"workspace"` for the channel; the `pairKeyOf` key for DMs. `#workspace` is always first and always present (even with zero messages); pairs follow most-recent-first. `unread` counts messages newer than `lastSeen[key]`; when a room has no `lastSeen` entry, the baseline is a `mountedAt` timestamp the caller captures once when the rail mounts (Mellow's plan-review F4: without it, a PAIR room that first appears mid-session would count as fully read and never badge). Net effect: no fake history badges at first open, but new rooms badge their new messages.
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
- Layout per proto `chats.tsx`, adapted by rulings: header row ("Chats" + total unread + maximize → `onOpenChat` + collapse), horizontal room-chip switcher (avatar, title, unread badge, live dot per R2; DM chips show a two-avatar overlapping cluster — reuse the Chat Hub sidebar's `-space-x-1` pattern, `ChatHub.tsx` pair rows — with an "A · B" title, not the proto's single first-member avatar), thin active-room meta line (members · N live), message stream (Today divider, consecutive same-sender grouping like the proto's `group()`; every message left-aligned — agents only per R8, there is no "mine" side; DM rooms render exactly like the channel, just filtered — Slack-style, do NOT port the Chat Hub's two-sided `pairKeyOf` pair view into the rail), near-bottom-guarded auto-scroll, and the read-only footer: *"Read-only live view — agents reply from their own terminal."*
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

- [ ] Step 1: Swap the mount; thread `statuses` from `tabs` — memoized (`useMemo` on `tabs`, like `roster`) so unrelated `WorkspacePane` state changes don't re-render the rail (plan-review F5).
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

- [ ] Step 1: Extract the SNAPSHOT STATE first (plan-review F2, blocking): move the drawer's snapshot block (`snapshots`, `hasHandoff`, `snapshotError`, `refetchSnapshots` with its seq guard, the `useSnapshotCreated` wiring — `ContextDrawer.tsx:281–332`) into a shared hook `useSessionSnapshots(sessionId)` in `src/lib/`. `WorkspacePane` calls it ONCE for the active session and passes the result to both bars — Task 5's Resume button gates on `hasHandoff` NOW, Task 6's popover consumes the same instance's `snapshots` later. Two separate hook calls = two fetchers = the risk-ledger violation this step exists to prevent.
- [ ] Step 2: Extract the Skills/Role/Session/Config code from `ContextDrawer.tsx` into `ContextTopBar` (move guards + comments); mount under the tab strip.
- [ ] Step 3: Verify `npx tsc --noEmit`; manual smoke: skills list + drift hint, restart/resume still work, popover open/close.
- [ ] Step 4: Commit.

### Task 6: Center bottom bar — Snapshots popover + handoff summary + meter chip

**Files:**
- Modify: `src/components/ContextBars.tsx`, `src/components/WorkspacePane.tsx`

**Interfaces:**
- `ContextBottomBar` props: same shape as top bar.
- Content per proto bottom section + R4: `Snapshots (N) ▾` trigger → popover with the drawer's full snapshot list (expand row → saved content, delete, submit-into-terminal, "Snapshot now", "Compact now") — move the logic verbatim; inline summary text `last handoff ~N tok · age`; Compact icon button; compact context-meter chip (R4) fed by the drawer's existing `session:context` estimate code, honest-labelled.
- Mount: bottom of the active agent's column, visually **above the composer** per proto (top bar / terminal / bottom bar / StdinBar). Exact DOM placement relative to the per-tab always-mounted layers is implementer judgment under two constraints: the terminal invariant (no remounting), and at most ONE live snapshot/meter fetcher at a time (don't duplicate per hidden tab).

- [ ] Step 1: Extract the snapshot popover UI + row actions + meter code into `ContextBottomBar`, consuming the `useSessionSnapshots` instance `WorkspacePane` already holds from Task 5 — this task adds NO new fetcher.
- [ ] Step 2: Mount per the constraint above.
- [ ] Step 3: Verify `npx tsc --noEmit`; manual smoke: snapshot list/expand/delete/submit, compact fires, meter updates, resume gating (Session's Resume enable still follows snapshot presence via the shared `useSessionSnapshots` state from Task 5).
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
- **`Resume` gating crosses bars**: the drawer computed `hasHandoff` once (`ContextDrawer.tsx:314`) and gated Session's Resume with it. Solved structurally by Task 5 Step 1 (`useSessionSnapshots`, single instance in `WorkspacePane`) — found blocking in plan review by Mellow (F2); do not regress to per-bar fetches.
- **No human sender-detection needed (R8)**: do not port `ChatView`'s user/assistant sides into the rail — the message table has no human identity, and inventing one would violate the honesty rule.
- **ChatHub refactor (Task 1) must be behavior-preserving** — it shipped reviewed at `c5465d1`; any visible change there is a defect.

## Smoke findings — 2026-07-04 human GUI smoke: FAIL (visual fidelity)

Human verdict on the installed build (relaunch 10:16): the rail "doesn't look like
the .arta design". Lead compared the live screenshot against the canonical proto
(`.arta/proto/screens/chats.tsx` + `theme.css`). Structure (header / chips / meta
line / stream / footer, popover bars) matches; the MESSAGE RENDERING does not.
Plan-attribution: Task 3 said "Layout per proto `chats.tsx`" but did not spell out
the bubble/header spec, and no design-acceptance gate existed before the human
smoke — that gap is the lead's; the guard below closes it.

**Findings (fix round 2, owner Dew, reviewer Mellow, design sign-off Arta):**

- **F-s1 — Bubble style.** Shipped (`ChatRail.tsx` message div): ChatHub carryover —
  `bg-overlay/[0.05]` + `borderLeft: 2px solid <sender color>`, effectively full-width.
  Canon (proto `.msg`, `theme.css:116` + `chats.tsx:104`): radius-md, `padding 0.5rem 0.72rem`,
  `1px solid` soft border, raised background, text `0.84rem/1.5`, container `max-width: 82%`,
  **no colored left border**. Map to real tokens per Global Constraints (app.css wins on VALUES;
  proto wins on STRUCTURE).
- **F-s2 — Per-message recipient rows (RULING R9).** Shipped renders a `→ recipient · reltime`
  line above every message in `#workspace`; the proto has no recipient line anywhere.
  R9: drop the arrow rows; recipient honesty moves to a `title` tooltip on the bubble
  (`→ <name>`). Broadcast fan-out (same text sent to 3 peers = 3 bubbles) is accepted
  Phase-1 data reality — do NOT dedup/merge; that would fabricate a channel-send that
  doesn't exist in the data.
- **F-s3 — Group header.** Proto (`chats.tsx:105-109`): one header per sender-group —
  `name · role · HH:MM` (absolute clock time of the group's first message; day context
  comes from the divider). Shipped: name only, relative times per message. Fix: header gains
  role + absolute `HH:MM`; per-message time rows go away with F-s2. `AgentIdentity`
  (`useWorkspaceChat.ts`) is `{name, color}` — extend it with `role` IF the hook's agent
  source already carries one; if it doesn't, escalate to lead before inventing a lookup.
- **F-s4 — Rail width.** Shipped `w-[300px]`; canon `w-[380px]` (proto `AppShell.tsx:167`).

**Confirmed-correct omissions (do not "fix"):** no filter button (R7), unread label hidden
at zero, no system pills (no system rows in `InterAgentMessage`), no typing indicator (R2).

**Timeline note:** proto `chats.tsx`/`data.ts` last edited 09:38, AFTER Task 3's commit
(09:36) — Arta CONFIRMED (with evidence, not defaulted) the 09:38 edit introduced no visible
element beyond F-s1..F-s4: `chats.full.png` (09:35, pre-edit) already renders the final design,
and the current proto maps 1:1 to the four findings (`chats.tsx:111` .msg / `:104` 82% /
`:107-108` role + absolute time / `AppShell.tsx` 380px). The proto's system pill is mock-data
only — stays a confirmed-correct omission.

**New guard — design-acceptance gate:** before ANY future human smoke of a lane that has
an `.arta/proto` design record, Arta pixel-reviews the actual rendering against the proto
and records PASS at `bb review:<lane>-design`. A lane without that key is not smoke-ready.

## Smoke round 2 — 2026-07-04 10:44: F-s1..F-s4 ACCEPTED · new human-directed finding F-s5

Human relaunched the fix-round-2 build (`966c4a0`, installed 10:36:47) and reviewed the
live rail. No finding was raised against F-s1..F-s4 — the human moved straight to
requesting an enhancement, so round-2 fixes are accepted. One new directive keeps the
lane open for fix round 3:

- **F-s5 — Per-message recipient chip (RULING R10, human-directed; supersedes R9's
  tooltip-only clause).** Human (10:44–10:45 screenshots + message): recipient must be
  visible WITHOUT hovering — "ขอ profile + name เล็กๆ บนหัว message ฝังขวา จะได้รู้ว่าส่งให้ใคร
  โดยไม่ต้อง hover". The rail's broadcast fan-out renders 3 identical bubbles with no
  visible recipient; tooltip-only honesty (R9) is not enough.
  Fix (`ChatRail.tsx` per-message loop, currently :328-343): each message row gains a
  small right-aligned recipient line ABOVE its bubble — recipient `Avatar` at size 4 +
  recipient name, `text-[10px] text-text-tertiary`, `justify-end` within the group's
  `max-w-[82%]` column. Keep the `title` tooltip (harmless redundancy). Reference look:
  the room-switcher DM chip (small avatar + name) the human pointed at in the 10:44
  screenshot — but no live dot and no pill border; this is metadata, not a control.
  R10 scope notes: (a) R9's NO-DEDUP clause still stands — fan-out stays 3 bubbles,
  each now labeled with its own recipient; (b) the proto has no recipient element at
  all, so this is a DELIBERATE human-directed delta from the proto — Arta's design
  gate must treat it as canon, and the proto should be updated to match so the design
  record stays truthful.

- **F-s6 — Always auto-scroll on new message (RULING R11, human-directed).** Human
  (10:52): "ตอนมี Message ใหม่ ให้ Auto scroll ไปข้างล่างด้วย". Shipped rail ports the
  Chat Hub's near-bottom guard (`ChatRail.tsx` snap effect, ~:165-189): a new message
  only snaps when the reader is already within 40px of the bottom. R11: the RAIL is a
  read-only live tail — a new message ALWAYS snaps to bottom, even mid-history-read
  (trade-off acknowledged and accepted by the human's directive). The Chat Hub keeps
  its guard — this ruling is rail-only.
  Fix: in the snap effect, `shouldSnap = forceScrollRef.current || isNew` (drop the
  `atBottomRef` condition); `atBottomRef`/`onStreamScroll` become dead — remove both.
  Mark-seen keeps keying off `shouldSnap` (auto-follow implies caught-up for the
  active room; other rooms still badge).
  Companion defect, same fix: collapsing then reopening the rail lands the stream at
  the TOP (fresh DOM node, `forceScrollRef` already false, `isNew` false). Add a
  `useEffect` on `open` that sets `forceScrollRef.current = true`, and include `open`
  in the snap effect's deps so reopening snaps to newest immediately.

Round-3 flow: Dew implements F-s5 (landed `f046b9e`) + F-s6 → Mellow re-reviews both →
Arta updates proto (F-s5 recipient chip; F-s6 is behavior-only, no proto change) +
design gate PASS at `bb review:right-rail-chats-design` → lead reruns tsc/vite →
rebuild, install, human re-smoke.
