# uishot: fail on swallowed [fixture] console errors

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop

## Problem

`pnpm uishot <view>` exits 0 when a component CATCHES a `[fixture] no handler
for command X` error and merely logs it via `console.error` — the script only
fails on `pageerror` and on a missing readiness sentinel. A view whose IPC
calls silently no-op therefore passes the pixel gate green while rendering
wrong/empty data (found by Tiësto; recorded as a known gap in CLAUDE.md and in
the uishot spec). Precedent detail lives in workspace memory 38930295: e.g.
WorkspacePane `instance.spawn` and `useSessionSnapshots` `snapshot.list`
caught the throw and uishot still exited 0.

## Fix

In `scripts/uishot.mjs` (and mirror in `scripts/uishot-eval.mjs` if it has the
same listener wiring):

1. Register `page.on('console', msg => ...)` and collect every message where
   `msg.type() === 'error'` OR `msg.text().includes('[fixture]')`.
2. After the screenshot is written, if the collected list is non-empty:
   print each offending line prefixed `[uishot] console-fail:` and exit 1.
   The screenshot file still gets written first — a failing run must leave
   the PNG on disk so the failure is inspectable.
3. Messages that are ordinary `console.error` forwards already printed by the
   script keep printing as today; the change is the exit code, not the log.
4. Do NOT fail on `console.warn` or lower — only `error`-type and `[fixture]`
   substring hits.

## Boundary

- scripts/uishot.mjs
- scripts/uishot-eval.mjs

## Verification (run all, record as gates)

1. Positive sweep — every view must still pass:
   `pnpm uishot home && pnpm uishot chat && pnpm uishot laneboard && pnpm uishot builder`
   (4 representative views; run the rest if any doubt).
2. Negative proof (NOT committed): temporarily comment out one fixture handler
   in `src/fixtures/scenarios/default.ts` that a caught call path uses, run
   `pnpm uishot home`, confirm exit 1 with the `[uishot] console-fail:` line,
   then `git checkout -- src/fixtures/scenarios/default.ts`. Paste the exit
   code + offending line into the READY note (the negative fixture edit is
   outside the boundary on purpose — it must never be committed).

## Risk ledger

- A stale vite server on :1420 serves ANOTHER checkout's code and uishot
  silently reuses it (memories cb04ff54 / 0a968062): before trusting any
  result, `lsof -nP -iTCP:1420 -sTCP:LISTEN` and kill servers whose cwd is
  not your worktree.
- Some views may ALREADY log error-type console messages in fixture mode
  (that is the known-gap this task closes). If the positive sweep fails on a
  pre-existing swallowed error, that is a REAL finding: report it in a task
  note (view, message) and escalate to Detoro for a ruling on whether the
  fixture handler gets added in-lane (boundary widening ruling) or as a
  follow-up task — do not silently allowlist it.
