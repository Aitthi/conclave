# Chat feed: verify sort order + attribution against shuffled fixture timestamps

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem

`.shots/home-default.png` showed chat bubbles in the sequence 16:22 → 15:12 →
18:05 — either the feed does not sort chronologically, or the fixture data was
authored in that order and the feed renders insertion order. Nobody has
verified which. This task settles it with evidence.

## Work

1. Read `src/lib/chatFeed.ts` (1.6K) — find where the feed assembles/sorts
   messages. Note: if any ordering compares ISO-8601 strings lexicographically,
   that is the bug class from memory 430bcbc8 (variable fractional-second
   widths break string compare) — parse then compare.
2. In `src/fixtures/scenarios/default.ts`, SHUFFLE the insertion order of the
   chat messages so that fixture-array order ≠ chronological order (keep the
   existing FIXED literal timestamps — never Date.now(); reordering array
   entries is enough, or swap which entry carries which timestamp).
3. Run `pnpm uishot home` and `pnpm uishot chat`, then OPEN each PNG with the
   Read tool (UI Pixel Gate, standing protocol in CLAUDE.md):
   - If bubbles render chronologically despite the shuffled array → the sort
     is real; keep the shuffled fixture as a permanent regression tripwire
     (comment in the fixture: "order intentionally shuffled — feed must sort")
     and report VERIFIED-OK.
   - If bubbles render in array order → fix the sort in `src/lib/chatFeed.ts`
     (chronological ascending by parsed timestamp), rerun both shots, confirm.
4. Same shots: verify bubble ATTRIBUTION labels (sender names) match what the
   fixture data says for each message — a label on the wrong bubble is a
   separate defect; fix only if the cause is inside `chatFeed.ts`, otherwise
   report with evidence and stop (likely a component bug outside boundary →
   Detoro rules on a follow-up task).

## Boundary

- src/lib/chatFeed.ts
- src/fixtures/scenarios/default.ts

## Verification (record as gates)

1. `pnpm build` (tsc must pass)
2. `pnpm uishot home`
3. `pnpm uishot chat`
Plus: attach `.shots/home-default.png` and `.shots/chat-default.png` paths in
the READY note after LOOKING at them (pixel gate — a green exit alone does
not count).

## Risk ledger

- Stale :1420 vite server serves another checkout (memories cb04ff54 /
  0a968062): `lsof -nP -iTCP:1420 -sTCP:LISTEN` before trusting shots.
- No frontend unit-test runner exists in this repo (scripts: only vite/tsc/
  uishot) — do not add vitest for this task; the shuffled fixture IS the
  regression guard.
- uishot exit 0 does not catch swallowed [fixture] console errors (open task
  uishot-console-fail) — grep the uishot console output for `[fixture]`
  before claiming green.
