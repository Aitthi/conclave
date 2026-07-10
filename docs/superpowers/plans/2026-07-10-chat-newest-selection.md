# ChatRail/ChatHub: newest-selection via parsed createdAt, not window position

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem (Dew escalation dd8dfa57 on chat-preview-order, verified by Detoro
## against the code before ruling)

Both chat components assume the raw message window is newest-first and pick
"newest" by position:

- `src/components/ChatRail.tsx:102` — `oldestFirst = [...roomMessages].reverse()`;
  `:125` `newest = oldestFirst[len-1]` (= raw `messages[0]`) feeds the
  autoscroll `isNew` trigger (`:127-131`) AND the `lastSeen` watermark
  (`:138-143`). With a shuffled window the watermark can store a non-newest
  createdAt → unread badges overcount and `isNew` misfires.
- `src/components/ChatHub.tsx:61` — same `reverse()`; `:92`
  `newest = visible[len-1]` feeds isNew/autoscroll snap (`:95`).

Rendering is already safe (`chatFeed.ts group()` sorts internally, c2665b8);
this task closes the LAST consumers of raw array order in the chat stack.

## Fix

1. Depends on chat-preview-order (lane 74b507a) being merged first — it
   exports `createdAtMs` from `src/lib/chatPairs.ts` for exactly this reuse.
2. In both components, select `newest` by `createdAtMs` maximum over the
   visible window (parse-then-compare; ties → keep current behavior of the
   later array element). Where the code needs oldest→newest ordering for
   group()/rendering, sorting by createdAtMs replaces the blind `reverse()`.
3. Preserve BOTH documented autoscroll contracts exactly: the rail is
   always-snap (R11, human-directed trade-off, comment at ChatRail.tsx:108),
   the hub is near-bottom-guarded. Only the *selection of newest* changes.
4. The lastSeen watermark stays an ISO timestamp (Mellow's F-a interface
   ruling, cited in the ChatRail comment) — store the createdAt of the
   createdAtMs-maximal message.

## Boundary

- src/components/ChatRail.tsx
- src/components/ChatHub.tsx

## Gates

pnpm build · pnpm uishot home · pnpm uishot chat
(UI Pixel Gate: read the PNGs; uishot exits 1 on console errors since 78d4b2c)

## Risk ledger

- The default fixture's message window is DELIBERATELY shuffled (regression
  tripwire) — badges/snap behavior under the shuffle is what this task makes
  correct; do not "fix" the fixture.
- Stale :1420 vite server from another checkout — lsof/kill first
  (CLAUDE.md known caveats).
- Do not touch src/lib/* here: if the exported helper is missing or wrong,
  escalate to Detoro instead of widening.
