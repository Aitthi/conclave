# Chat Hub redesign — align the full chat page to the rail canon

**Date:** 2026-07-04 · **Lead:** Detoro (bfb737ff) · **Implementer:** Dew ·
**Reviewer:** Mellow · **Design gate:** Arta · authority: in-loop (design
human-directed: "Re-design Full chat page ด้วย", prioritized 11:07+ "ทำ Full
page chat ก่อนนะ ที่ค้างอยู่")

**Design source of truth:** `.arta/proto/screens/chat-hub.tsx` @ b29b8a3
(Arta, pass 1 — accepted as the implementation basis when the human ordered
the lane built). Rail canon precedents live in
`docs/superpowers/plans/2026-07-04-right-rail-chats.md` (R1–R11).

**Target file:** `src/components/ChatHub.tsx` (267 lines). Do NOT touch
`ChatRail.tsx` rendering/behavior (rail already gated PASS) beyond the pure
helper extraction in Task 1.

## Rulings (recorded here; Arta gate must treat these as canon)

- **R-hub-1** (already on bb): hub adopts the rail visual canon — `.msg`
  bubbles (no colored side stripe), sender-group headers name·role·HH:MM, day
  dividers, recipient chip below bubble (R10-amended), lowercase queued
  label. Hub KEEPS its near-bottom scroll guard (R11 is rail-only) and keeps
  sidebar + search + All/pair structure.
- **R-hub-2 (new):** the `injected` label (`m.autoSubmitted`) stays in the
  hub meta rows, next to `queued`. The proto's sample data simply doesn't
  model auto-submission; dropping real message state would be an
  information regression, not a design decision.
- **R-hub-3 (new):** sidebar pair-row timestamps keep `timeHint(...)`
  (relative). Precedent: the queued-label ruling — app convention is canon
  for elements that predate the redesign, and a bare absolute HH:MM on a row
  that can be days old would lie. (Proto shows raw HH:MM; recorded
  deviation.)
- **R-hub-4 (new):** app material conventions win over proto surface colors:
  header icon chip stays `bg-ink text-on-ink` (matches Blackboard header),
  the hub sidebar keeps the app surface (no distinct `--color-sidebar`
  fill), and sidebar pair avatars follow the rail's `RoomAvatar` pattern
  (`-space-x-1`, no ring shadow). Structure/sizes still follow the proto.

## Global constraints (every task inherits)

- Language: all UI copy English. No new dependencies. No IPC/wire changes —
  this is render-layer only; `useWorkspaceChat`, `derivePairs`, `pairKeyOf`
  stay as-is.
- Verification gate per commit: `npx tsc --noEmit` exit 0 (repo has
  `noUnusedLocals`), `pnpm vite build` succeeds.
- Size mapping convention (rail precedent): proto rem values map to the
  nearest app Tailwind class; sub-pixel drift is acceptable and Arta judges
  fidelity on structure + tokens, not px-exactness.
- Colors: proto `faint` → `text-text-tertiary`, `heading` →
  `text-text-primary` semibold, `--color-border` → `border-overlay/[0.06]`,
  `--color-queued` → `text-warning`, bubble = exactly the rail's classes.

## Task 1 — extract shared feed helpers (pure move, rail unchanged)

Create `src/lib/chatFeed.ts` exporting, moved VERBATIM from
`src/components/ChatRail.tsx`:

- `clockLabel(iso: string): string` (ChatRail.tsx:79-83)
- `dayLabel(iso: string): string` (ChatRail.tsx:87-96)
- `group(messages: InterAgentMessage[]): MsgGroup[]` + the `MsgGroup`
  interface (ChatRail.tsx:101-113)

Update `ChatRail.tsx` to import them (delete the local copies; keep its
comments by moving them along). `ChatHub.tsx` imports the same. No behavior
change in the rail — a diff of its rendered output must be empty.

## Task 2 — All feed (ChatHub.tsx:187-217)

Replace the per-message `from → to` rows with the proto's grouped feed
(proto chat-hub.tsx:129-153):

- Order: keep `visible` oldest-first as today; `const groups =
  useMemo(() => group(visible), [visible])`.
- Day divider before group `gi` when `gi === 0` or the `dayLabel` of the
  previous group's first item differs (rail pattern ChatRail.tsx:295-311).
  Divider style at hub scale: `h-px flex-1 bg-overlay/[0.06]` lines, label
  `text-[10.5px] text-text-tertiary font-mono tabular-nums`.
- Group row: avatar + column, `max-width: 72%` on the column (proto :138).
  Avatar: extend the LOCAL `Avatar` in ChatHub.tsx with `size?: 4 | 5 | 7`;
  `7` → `w-7 h-7 text-[12px] rounded-[8px]` (proto av-md 28px). All-feed
  groups use size 7.
- Group header (proto :139-143): name `text-[12.5px] font-semibold
  text-text-primary`, role `text-[10px] text-text-tertiary`, time
  `clockLabel(first.createdAt)` `text-[10px] text-text-tertiary font-mono
  tabular-nums`, `flex items-baseline gap-2`. Drop the `MoveRight` icon and
  the `→ to` header (recipient moves to the chip). Remove the now-unused
  `MoveRight` import.
- Each message in the group (proto :144-149): bubble div with EXACTLY the
  rail bubble classes `rounded-md border border-overlay/[0.06]
  bg-surface-raised px-[0.72rem] py-2 text-[0.84rem] leading-[1.5]
  text-text-primary`, `title={'→ ' + to.name}`, content via `<ClampText
  text={m.text} outgoing={false} lines={12} />`.
- Meta row BELOW each bubble (proto MetaRow :41-55), `flex items-center
  gap-1 self-stretch`: `queued` label first when `m.status === "queued"`
  (`text-[9px] text-warning`), `injected` when `m.autoSubmitted`
  (`text-[9px] text-text-tertiary`, R-hub-2), time
  `clockLabel(m.createdAt)` `text-[10px] text-text-tertiary font-mono
  tabular-nums`, then recipient chip pushed right via `ml-auto`:
  `<Avatar identity={to} size={4} />` + name, `text-[10px]
  text-text-tertiary`, `title={'→ ' + to.name}` (rail ChatRail.tsx:332-340).

## Task 3 — Pair view (ChatHub.tsx:218-261)

Rebuild on the same grouped feed (proto chat-hub.tsx:158-180):

- Keep the existing side rule: lexicographically-first id of the pair key
  renders LEFT (`leftIdOfPair`), the other RIGHT. Group with `group()` too —
  `const groups = group(visible)` works for both views; compute once.
- Group container: `flex flex-col gap-1 ${onLeft ? "items-start" :
  "items-end"}`, day dividers same rule as Task 2.
- Group header (proto :164-168): `<Avatar identity={from} size={5} />` +
  name `text-[11px] font-semibold text-text-primary` + time
  `clockLabel(first.createdAt)` `text-[10px] text-text-tertiary font-mono
  tabular-nums`, `flex items-center gap-1.5`.
- Messages: same rail bubble classes as Task 2 (NO side stripes, NO
  asymmetric `rounded-bl-md`/`rounded-br-md`, no per-side border color),
  wrapped in `flex flex-col gap-0.5 ${onLeft ? "items-start" : "items-end"}`
  with `max-width: 72%`.
- Meta row (proto :172-175): time + `queued` + `injected` (R-hub-2),
  `text-[9px]`/`text-[10px]` as in Task 2, aligned to the message's side
  (no recipient chip — the pair implies it; proto :156-157).

## Task 4 — Sidebar + header (ChatHub.tsx:104-171)

- Sidebar width `w-[240px]` (proto :100; was 220).
- `All` row (proto :101-107): add a right-aligned total count
  `<span className="ml-auto text-[10px] text-text-tertiary font-mono
  tabular-nums">{messages.length}</span>`.
- Section label between All and the pairs (proto :108): `Conversations` —
  `px-2 pt-2 pb-1 text-[10px] font-bold tracking-wider text-text-tertiary
  uppercase` (matches Roster's section-label convention, Roster.tsx:444).
- Pair rows: keep current structure (Avatar(4) pair with `-space-x-1`,
  R-hub-4), name `text-[12px] font-medium truncate`, keep
  `timeHint(p.lastAt)` (R-hub-3).
- Header: search box widens to `w-52` (proto :89; was `w-44`). Everything
  else in the header stays (R-hub-4: bg-ink icon chip, read-only badge).

## Out of scope / do-not-touch

- Scroll behavior block (ChatHub.tsx:69-97) — the near-bottom guard stays
  EXACTLY as-is (R11 is rail-only). The `visible`-driven effect keys off the
  same `visible` array; grouping happens after it and must not change the
  effect's deps or semantics.
- `useWorkspaceChat`, `chatPairs.ts`, `chatRooms.ts`, IPC, Rust — untouched.
- Empty/error states (:179-186) keep their current copy and styling.

## Risk ledger

- `noUnusedLocals` will fail the gate if `MoveRight` (and possibly
  `timeHint` if you accidentally drop its remaining use) linger unused —
  clean imports as you go.
- The rail imports moved helpers after Task 1; typos there break BOTH
  components. Run `npx tsc --noEmit` after Task 1 alone before continuing.
- `group()` is typed on `InterAgentMessage`; ChatHub's messages come from
  the same `useWorkspaceChat`, so no type bridging should be needed — if tsc
  disagrees, escalate to the lead rather than widening types.
- Pair view previously rendered per-message headers; after grouping, a long
  monologue renders ONE header + N bubbles. That is the proto's intent, not
  a regression.

## Gate chain

1. Dew: implement Tasks 1-4 (one commit per task or one clean commit — Dew's
   call), `npx tsc --noEmit` exit 0 + `pnpm vite build` green, update
   `progress:chat-hub-redesign`.
2. Mellow: code review all hub commits together vs this plan.
3. Arta: design gate vs proto b29b8a3 with R-hub-2/3/4 as recorded canon —
   verdict to `bb review:chat-hub-design`.
4. Lead: rerun gates, build + install, human smoke.

Escalations: design/spec conflicts → Detoro (final). Implementation judgment
within the plan → Dew, logged in `progress:chat-hub-redesign`.
