# Chat Hub — workspace-wide inter-agent chat view (Phase 1: read-only)

**Date:** 2026-07-03 · **Status:** approved by the human overseer
**Supersedes (partially):** the Messages timeline inside `ContextDrawer.tsx`
(commit 0992bb5) — that timeline moves here; the drawer keeps only the policy
header.

## Problem

The inter-agent conversation is the human's main supervision surface, but it
lives in a 306px drawer section capped at `max-h-72`: long messages need
"Show more" constantly, the timeline competes with five other sections, and
most of the drawer below it sits empty (see the 2026-07-03 screenshot review).
Reading a multi-paragraph agent exchange there is painful, and the view is
per-agent only — supervising N agents means switching N tabs.

## Decisions (settled with the human, 2026-07-03)

1. **Placement:** a dedicated, workspace-level Chat view in the center pane —
   NOT a bigger drawer section, NOT a per-agent tab.
2. **Perspective:** workspace-wide hub. All pairwise conversations in one
   place, plus a merged "All" feed. Not scoped to the selected agent.
3. **Composing:** Phase 1 is read-only (read + search + filter). A human
   composer is a recorded follow-up, out of scope here: today's message model
   requires `from_instance_id` to be a real workspace agent, so human-origin
   messages need a data-model change that must not block the readability win.

## Architecture

### Navigation (existing pattern, zero new machinery)

`AppShell.tsx` already switches the center pane to a workspace-level screen
for the Blackboard (`showBlackboard`, AppShell.tsx:197). Chat Hub clones that
exact pattern:

- `showChat` state in AppShell; opening one of Blackboard/Chat closes the
  other (they share the center pane).
- A Chat toggle button (MessageSquare icon) in the same header cluster as the
  Blackboard toggle, enabled only when a workspace is active.
- `<ChatHub key={workspaceId} workspaceId onClose />` renders in the center
  pane in place of `WorkspacePane`. Terminals survive unmounting because PTY
  sessions are backend-owned — the same trade the Blackboard already makes.

### Backend — one new query

`message.listForWorkspace { workspaceId, limit? }`:

- Repo fn `inter_agent_message::list_for_workspace(db, workspace_id, limit)`
  — messages whose `from_instance_id` AND `to_instance_id` both belong to the
  workspace (join `workspace_agent`), newest-first, `limit` default 200,
  clamped to the existing max.
- Command validates the workspace exists (`NotFound` otherwise), mirrors
  `message.list`'s shape.
- Rust tests per convention: workspace scoping (a message in another
  workspace never leaks in), ordering, unknown-workspace `NotFound`, limit
  clamp.
- Frontend: `ipc` entry + type, mirroring `message.list`.

### Frontend — `src/components/ChatHub.tsx`

Two-pane layout, full center pane:

**Sidebar (left, ~220px):**
- "All" entry, then one entry per distinct unordered pair `{A, B}` derived
  from the loaded messages, ordered by most-recent activity. Names/colors
  resolved via the workspace roster (fallback: raw id + neutral grey, same as
  the drawer does today).
- A client-side text search box filtering the visible timeline (message text,
  sender/recipient names). Phase 1 searches the loaded window only — no
  server-side search.

**Timeline (right):**
- **All view — feed style.** Every row left-aligned: color avatar, a
  `Sender → Recipient` header, text, timestamp + status hints (`queued`,
  `injected`). In a hub there is no "self", so bubble left/right alignment is
  meaningless — a feed reads honestly.
- **Pair view — conversation style.** Bubbles left/right with a stable side
  per participant (deterministic: lexicographically-first instance id on the
  left), peer name + color accent on each side's first-of-run.
- Long text: measured line-clamp with Show more/less (the drawer's
  `ClampText`), at a roomier ~12 lines given the wide pane.
- Live updates: refetch on inter-agent-message events (the existing injection
  event bus), guarded by the same monotonic-seq + mounted-ref discipline as
  `ContextDrawer`. Auto-scroll snaps to newest only when the reader is near
  the bottom or on view switch (near-bottom guard ported as-is).

### `ContextDrawer.tsx` — Messages section slims down

The timeline, peer filter chips, ClampText, and the auto-scroll machinery
leave the drawer. The section keeps:
- the compact policy header (Accepts-from · Auto-submit), unchanged;
- an "Open chat" affordance that opens the hub (wired via a new optional
  `onOpenChat` prop threaded from AppShell).

Net: the drawer loses ~150 lines and the screenshot's dead space; the
conversation gains a real surface.

## Error handling

- `message.listForWorkspace` failures: dev-console error + inline "Couldn't
  load messages" note in the hub (same posture as the drawer's fetch errors —
  never silently empty).
- Peer no longer in the roster: render raw id + neutral grey (existing
  fallback), never drop the message.

## Testing

- **Rust:** repo + command tests listed above.
- **Frontend:** follows the repo's existing practice for view components
  (no component test harness exists today; the pair-derivation and
  side-assignment helpers should be written as pure functions so they are
  trivially testable if/when one lands).

## Out of scope (recorded for later)

- Human composer (Phase 2) — needs human-origin in the message model.
- Server-side search / pagination beyond the recent-200 window.
- Unread markers, notification badges.
