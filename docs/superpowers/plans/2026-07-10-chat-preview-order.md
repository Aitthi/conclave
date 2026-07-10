# Chat previews/autoscroll: stop consuming raw array order

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem (follow-up from task chat-feed-order-check-v2, merged c2665b8)

The chat FEED now sorts by parsed createdAt inside `chatFeed.ts group()`, and
the default fixture intentionally returns messages in non-chronological order
as a regression tripwire. But Arta's lane verified only rendering: ChatRail's
newest/deriveRooms previews and ChatHub's derivePairs still consume the RAW
array order, so conversation previews, "newest" selection, and autoscroll
targets can point at the wrong message when the backend/fixture array is not
chronological. Also: `src/fixtures/scenarios/data.ts` carries a stale
'newest-first' comment above a messages array that is not sorted.

## Work

1. Read `src/lib/chatRooms.ts` (deriveRooms) and `src/lib/chatPairs.ts`
   (derivePairs): wherever "latest message" / preview / ordering is derived
   from array position (first/last element), derive it from parsed createdAt
   instead (parse-then-compare — never lexicographic string compare, memory
   430bcbc8). Reuse one shared helper if that stays inside these two files;
   do not create new modules.
2. Fix the stale 'newest-first' comment in `src/fixtures/scenarios/data.ts`
   to state the truth: order is intentionally NOT chronological (tripwire).
3. If the defect turns out to live in a COMPONENT (ChatRail.tsx / ChatHub
   consuming positions directly, autoscroll math), do not chase it out of
   boundary: report file+line evidence in a task note and escalate to Detoro
   for a follow-up ruling.
4. Verify with the shuffled default fixture: `pnpm uishot home` and
   `pnpm uishot chat`, then READ the PNGs (UI Pixel Gate): conversation list
   previews must show each pair's chronologically-newest message; the feed
   stays chronological. Note: uishot now exits 1 on error-type/[fixture]
   console messages (78d4b2c) — a red gate here is signal, not noise.

## Boundary

- src/lib/chatPairs.ts
- src/lib/chatRooms.ts
- src/fixtures/scenarios/data.ts

## Gates

pnpm build · pnpm uishot home · pnpm uishot chat
(+ attach shot paths in the READY note after looking at them)

## Risk ledger

- Stale :1420 vite server from another checkout serves the wrong code —
  lsof/kill first (CLAUDE.md known caveats; hit three times on 2026-07-10).
- data.ts is shared fixture DATA for every view — do not reorder or edit its
  entries; only the comment changes there. The deliberate shuffle lives in
  default.ts's message.listForWorkspace and must stay.
