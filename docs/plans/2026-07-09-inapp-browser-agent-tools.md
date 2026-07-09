# Plan: InAppBrowser agent tools

Date: 2026-07-09
Owner: Aoki d1a70cab-1d34-490c-9fd7-ba4d4c940606
Authority: in-loop
Requester intent: "ทำ InAppBrowser พร้อม Tools ไว้ใช้ Agent ไว้ใช้ได้ไหม จะได้ไม่ต้องไป run Playwright"

Amendment 2026-07-09: original task slug `inapp-browser-agent-tools` was
abandoned after a second stall with no lane diff to preserve. Recovery task
uses slug `inapp-browser-agent-tools-recovery`. Gate examples below use
`<task-slug>` intentionally.

## Decision

Ship a V1 Conclave-managed browser session that agents can drive through the
existing `conclave` CLI/UDS tool surface. The normal path must not require an
agent to run Playwright/Puppeteer. The browser lives inside the Conclave app
process as a Tauri `WebviewWindow`; the agent controls it through explicit
browser commands.

V1 scope:

- `conclave browser open <url>` creates or focuses the browser window.
- `conclave browser goto <url>` navigates the current browser window.
- `conclave browser status` returns current URL/title when available.
- `conclave browser snapshot` returns a JSON DOM/text snapshot useful to an
  agent: URL, title, body text excerpt, headings, links, inputs, buttons, and a
  generated selector for actionable elements.
- `conclave browser click <selector>` clicks an element in the page.
- `conclave browser type <selector> <text...>` focuses and fills/types into an
  input-like element.
- `conclave browser eval <js...>` exists as an escape hatch for debugging and
  returns the JSON-serialized result.
- A Browser rail action opens a compact in-app control view for human visibility
  and status, matching the existing macOS-style shell.

Non-goals for V1:

- No Playwright/CDP dependency in the normal command path.
- No full browser automation framework.
- No guaranteed pixel screenshot of arbitrary remote pages. Tauri/WebKit exposes
  eval/navigation cleanly, but this repo does not currently have a native
  screenshot surface equivalent to CDP. If pixel capture becomes required, split
  it into a follow-up with a documented permission/dependency decision.
- No credential vault or cookie export.

## Design Ruling

Use one deep module:

`runtime::browser` owns all WebView lifecycle and page-tool behavior behind a
small interface:

- `open(app, url) -> BrowserState`
- `goto(app, url) -> BrowserState`
- `status(app) -> BrowserState`
- `snapshot(app) -> BrowserSnapshot`
- `click(app, selector) -> BrowserActionResult`
- `type_text(app, selector, text) -> BrowserActionResult`
- `eval_json(app, js) -> Value`
- `close(app) -> BrowserState`

Callers must not build JS snippets inline. `commands::browser` parses payloads,
calls `runtime::browser`, and serializes results. `cli::map_argv` maps shell
verbs to router commands. Frontend views call the same router commands through
`src/ipc/commands.ts`.

Why this seam: WebView creation, async `eval_with_callback`, timeout handling,
selector escaping, and DOM snapshot JS are fragile and must stay local. Deleting
this module would spread that complexity through CLI mapping, IPC handlers, and
React.

## Files

Expected implementation files:

- `src-tauri/src/engine/runtime/browser.rs`
- `src-tauri/src/engine/runtime/mod.rs`
- `src-tauri/src/engine/commands/browser.rs`
- `src-tauri/src/engine/commands/mod.rs`
- `src-tauri/src/engine/router.rs`
- `src-tauri/src/engine/commands/cli.rs`
- `src-tauri/src/bin/conclave-cli.rs`
- `src-tauri/src/engine/bus.rs` and `src/ipc/events.ts` only if the UI needs a
  live `browser:changed` refresh event; prefer polling/status refresh if enough.
- `src/ipc/types.ts`
- `src/ipc/commands.ts`
- `src/components/Rail.tsx`
- `src/components/AppShell.tsx`
- `src/components/InAppBrowserView.tsx`
- `src/fixtures/scenarios/default.ts`
- `src/fixtures/scenarios/empty.ts`
- `src-tauri/skills/tool-map/SKILL.md`
- this plan file

Do not touch `scripts/uishot.mjs` except for a narrow compatibility bug. The new
browser tools are not a replacement implementation of uishot; they are an
interactive agent-visible browser.

## CLI Contract

Help row:

`browser       open|goto|status|snapshot|click|type|eval|close`

Suggested syntax:

- `conclave browser open <url>`
- `conclave browser goto <url>`
- `conclave browser status`
- `conclave browser snapshot [--max-text N]`
- `conclave browser click <selector>`
- `conclave browser type <selector> <text...>`
- `conclave browser eval <js...>`
- `conclave browser close`

Output is JSON for `status`, `snapshot`, and `eval`. Action commands print a
compact JSON result with `ok`, `url`, and optional `message`. CLI strings are
English.

`open` and `goto` must normalize missing schemes to `https://` except for
`about:blank`, `http://`, `https://`, and `file://`. Invalid URLs fail loudly.

## Runtime Notes

- Browser label is stable: `agent-browser`. V1 is a single shared browser per
  Conclave app process.
- Use `state.app()` and fail with a clear error when no `AppHandle` exists
  (unit tests can cover pure helpers without opening a real WebView).
- Create the window with `tauri::WebviewWindowBuilder` and `WebviewUrl::External`.
- Use `WebviewWindow::navigate` for later navigation.
- Use `WebviewWindow::eval_with_callback` plus a bounded Tokio oneshot timeout
  for commands that need return values.
- JS snippets must catch their own exceptions and return JSON with `{ ok:false,
  message }` rather than hanging the callback.
- Snapshot should cap body text by `maxText` to avoid flooding an agent's
  context. Default 12000 chars, hard max 50000.
- Selectors emitted by snapshot must be usable by `click` and `type` without the
  agent reverse-engineering DOM paths.

## UI Notes

Add a Browser rail action using a lucide browser/globe-style icon. The Browser
view is a center-pane destination, not a marketing page. It should show:

- compact header with URL input, back/forward/reload/open buttons if they are
  cheap; otherwise URL input + open/status is enough for V1;
- current URL/title/status;
- latest snapshot summary in a dense inspector panel;
- no nested cards and no explanatory feature copy.

If the real WebView is a separate Tauri window in V1, the React view is a control
surface and status inspector; it must say nothing about implementation details
in visible copy. Opening the view should call `browser.open` with `about:blank`
or focus the existing browser window.

Fixture mode must not call Tauri browser APIs. Add fixture handlers returning a
fixed literal browser status/snapshot with fixed timestamps only.

## Tests And Gates

Backend:

- Unit-test URL normalization.
- Unit-test CLI mapping for every `browser` subcommand, including bad args.
- Unit-test snapshot/action JS builder helpers where practical.
- Run `cd src-tauri && cargo test browser`.
- Run `cd src-tauri && cargo test`.

Frontend:

- Run `pnpm build`.
- Because this lane touches `src/` UI, run UI Pixel Gate before READY:
  - `pnpm uishot home`
  - `pnpm uishot home --scenario empty`
  - if a hash-routed `browser` view id is added to fixture routing, run
    `pnpm uishot browser` and `pnpm uishot browser --scenario empty` instead
    of overloading home.
- Open and visually inspect every `.shots/*.png` produced. Attach paths in the
  READY note.
- Record each shot with `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 <task-slug> -- pnpm uishot <view>`.

Manual/live verification after app rebuild:

- Launch or use a running Conclave app.
- `conclave browser open https://example.com`
- `conclave browser snapshot`
- `conclave browser eval document.title`
- `conclave browser close`

## Risks

- WebView screenshot support is not available through the current Tauri surface.
  Do not fake it. V1 snapshot is DOM/text, not pixels.
- `eval_with_callback` can fail silently if JS throws outside the wrapper. Keep
  every injected script self-contained and exception-safe.
- Remote pages can block or mutate DOM under automation. Commands should time
  out and return clear JSON errors.
- Same-user local CLI trust model already applies to UDS. Browser `eval` is a
  powerful local tool; do not expose it over any network or plugin passthrough.
- UI Pixel Gate cannot exercise real native WebView in fixture mode. The fixture
  validates the React control surface only; live browser behavior is covered by
  CLI/manual verification after rebuild.

## Acceptance

The lane is READY only when:

- `conclave browser ...` verbs exist in help and route through the allowlisted
  CLI path.
- A real app can open/navigate a Tauri browser window and return a DOM snapshot
  without Playwright/Puppeteer.
- Browser control UI builds and passes the UI Pixel Gate with inspected PNGs.
- `src-tauri/skills/tool-map/SKILL.md` teaches the new verb.
- All listed backend and frontend gates are recorded on the task.
