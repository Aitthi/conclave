# Conclave (codeup) — Agent Instructions

## UI Pixel Gate (STANDING PROTOCOL — Detoro ruling, 2026-07-05)

Every lane that touches `src/` UI must, **before** marking the task READY:

1. Run `pnpm uishot <view>` for **each affected view** (repeat with `--scenario empty` when the change affects empty states).
2. **OPEN and LOOK at each PNG** with your image-capable file reader (Read tool on the `.shots/*.png` path). A green exit code alone does not count — you must see the pixels.
3. Attach the shot paths in the READY task note.
4. Record the run via `conclave task gate <ws> <slug> -- pnpm uishot <view>`.

The human's manual checklist remains the final acceptance; this gate catches broken UI before it ever reaches them.

## How to see the UI you built

`pnpm uishot <view> [--scenario default|empty] [--full]`

- Renders the **real `src/` app** (not a mockup) in headless Chrome at `http://localhost:1420/?fixture=<scenario>#view=<viewId>`.
- View ids: `home laneboard memory artifacts blackboard chat library builder settings`.
- Waits for the readiness sentinel `body[data-conclave-ready="1"]`, then writes `.shots/<view>-<scenario>.png` (2880x1800).
- Exits 1 on any `pageerror` and forwards `console.error` to your terminal.
- Then **Read the PNG file** to actually inspect the result.

### Fixture mode (how the app runs without Tauri)

- Single IPC seam: `src/ipc/commands.ts` `call()` and `src/ipc/events.ts` `useEvent()` route to `src/fixtures/` when DEV + `?fixture=` is present. Prod builds tree-shake all of it.
- Add handlers in `src/fixtures/scenarios/*.ts`, typed off the `Commands` map. **Fixed literal timestamps only** (no `Date.now()`).
- A missing fixture handler THROWS `[fixture] no handler for command X` — loud by design; add the handler, never swallow it.

### Known caveats

- PTY/terminal panes render empty in fixture mode — accepted, not a bug.
- `uishot` exit code does NOT yet catch console-level `[fixture]` errors that components swallow (open task `uishot-console-fail`). If your view calls IPC commands, grep the uishot console output for `[fixture]` before claiming green.
- Any `@tauri-apps/api` getter on the render path can throw **synchronously** in plain Chrome (`__TAURI_INTERNALS__` missing). A promise `.catch()` will not save you — wrap the getter call itself in try/catch (see `src/lib/fileDrop.ts`).

## Pointers

- Spec: `docs/superpowers/specs/2026-07-05-uishot-real-pixels-design.md`
- Plan: `docs/superpowers/plans/2026-07-05-uishot-real-pixels.md`
- Blackboard key: `protocol:ui-pixel-gate`
