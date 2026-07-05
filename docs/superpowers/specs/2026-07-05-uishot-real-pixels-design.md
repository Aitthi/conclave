# uishot: real-pixel feedback loop for UI lanes (+ Design view screen persistence)

Date: 2026-07-05
Owner: Detoro (lead, bfb737ff) · authority: in-loop
Requester: human — "ระบบ Design จากการใช้จริง": agents never see rendered pixels of the
real app they change (unlike Arta's viewer loop), so broken UI ships; and the Design
view forgets which screen the human was viewing on every update.

## Problem

1. Every design pipeline in this repo verifies mockups only: `.arta/proto/screens`
   (Arta canvas) or `<ws>/design/screens` (design-host, port 7343). Nothing renders or
   screenshots the shipped UI in `src/`. `conclave design review` is pure static
   analysis. Policy r13 (ruling 35968ae3) substitutes a human checklist because "live
   UI proof is env-blocked for agents". Result: layout/runtime breakage in `src/`
   passes every gate until a human eyeballs it.
2. `design-host/src/Shell.tsx:15` keeps the active screen in bare `useState`. Any
   full-reload (agent edits a screen file → Vite reload) resets the viewer to the
   default screen, dropping the human's place.

## Decisions (settled with the human, 2026-07-05)

- **Scope**: phase 1 = render the real `src/` app + screenshot for agent self-check.
  Real-usage capture of the running Tauri window (human sessions feeding design canon)
  is a possible later phase, out of scope here.
- **Enforcement**: hard gate. Every UI lane must run uishot via `conclave task gate`
  and the implementer must view the PNG before marking READY. r13 human checklist
  remains the final acceptance layer, no longer the only pixel check.

## Rejected alternatives

- **Per-component story harness** (storybook-style mounts of `src/` components with
  fixture props): rejected — per-component wiring cost, and it misses
  layout-in-context bugs (the StdinBar-class failures that actually shipped).
- **Capturing the real running Tauri window** (macOS window capture): most realistic
  but automation-hostile — needs a GUI app running, capture permissions, and offers no
  deterministic state control. Deferred to a later phase, not phase 1.

## Design

### 1. Fixture mode (one seam)

`src/ipc/commands.ts` and `src/ipc/events.ts` are the only modules importing
`@tauri-apps/api` (plus two leaf utils: `src/lib/theme.ts`, `src/lib/fileDrop.ts`).
Add a fixture layer behind that seam:

- Activation: URL query `?fixture=<scenario>` AND `import.meta.env.DEV`. Production
  builds never include the branch (guarded so it dead-code-eliminates).
- `src/fixtures/` holds a typed handler map keyed by the `Commands` interface, plus a
  scenario dataset with **fixed timestamps** (reproducible screenshots). Event
  subscriptions (`events.ts`) become no-ops or scripted emitters per scenario.
- v1 scenarios: `default` (populated workspace: agents, chats, tasks, blackboard) and
  `empty` (fresh install look).
- An invoke against a command with no fixture handler **throws loudly** with the
  command name — a visibly broken screen, never a silent fallback.
- Readiness sentinel: the app sets `data-conclave-ready="1"` on `<body>` once initial
  fixture data has loaded and the first paint of the routed view is done (mirrors
  Arta's `data-arta-ready` contract in `proto/shell/bridge.ts`).

### 2. `uishot` capture CLI

Port Arta's proven capture core (`/Users/detoro/code/arta/vite/headless-snapshot.ts`:
`findChrome`, puppeteer-core launch/reuse, viewport @2x, full-page UNCLAMP trick) into
`scripts/uishot.mjs`. Only Arta couplings to replace: URL builder and the readiness
selector.

- CLI: `pnpm uishot <route> [--scenario default|empty] [--full] [--out <path>]
  [--viewport WxH]` (default 1440x900 @2x).
- Ensures the Vite dev server on 1420 (reuse if running, else start with a timeout),
  navigates to `http://localhost:1420/?fixture=<scenario>#<route>`, waits for
  `body[data-conclave-ready="1"]`, writes PNG to `.shots/<name>.png` (gitignored).
- Also forwards `console.error` / `pageerror` from the page to stdout and exits
  non-zero on page crash — the agent sees runtime breakage, not just pixels.
- `puppeteer-core` lands as a root devDependency; no browser download — discovers the
  installed Chrome (env override `CHROME_PATH` / `PUPPETEER_EXECUTABLE_PATH`).

### 3. Gate + protocol wiring

- UI lanes run `conclave task gate <ws> <slug> -- pnpm uishot <route>` per affected
  route (exit code + SHA pinned on the ledger).
- Implementer skill sidecar amendment: before setting READY on a lane touching
  `src/` UI, the implementer must (a) run uishot on every affected route, (b) open and
  actually look at the PNG(s), (c) attach the shot paths in the READY note.
- Policy record update: r13 checklist stays as final human acceptance; the
  "env-blocked" rationale is superseded for anything reachable via fixture mode.

### 4. Design view screen persistence

`design-host/src/Shell.tsx`: persist the active screen to the URL hash
(`#/<screen-id>`) and localStorage keyed per project; restore on mount with
precedence hash → localStorage → default. Keep the existing guard (line 18) that
falls back when the persisted screen no longer exists.

## Testing

- uishot smoke eval: capture a main route in both scenarios, assert PNG exists with
  expected dimensions and zero pageerrors (wire into the existing evals pattern under
  `design-host/evals/` or a sibling `scripts/` eval).
- Fixture handlers typed against the `Commands` map so payload drift is a compile
  error; missing-handler behavior covered by a unit test.
- Shell persistence: manual check (edit a screen file → full reload → same screen) +
  a small unit test on the restore-precedence helper.

## Risk ledger

- Views bound to real PTY/terminal state (xterm scrollback) will render as empty
  frames in fixture mode — accepted in v1; note it in the gate skill so agents don't
  chase phantom bugs.
- Machines without Chrome: uishot fails with a clear `CHROME_PATH` hint (same
  discovery contract as Arta).
- Fixture drift vs real backend: fixtures typed off `Commands` catches shape drift,
  not semantic drift; r13 human acceptance remains the backstop.
- Vite 1420 is `strictPort: true`; a stale dev server from another lane is reused,
  which may serve stale code — uishot prints the server PID/start mode so the agent
  can tell reuse from fresh-start.
