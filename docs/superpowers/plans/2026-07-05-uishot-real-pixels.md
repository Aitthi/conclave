# uishot Real-Pixel Feedback Loop Implementation Plan

> **For agentic workers:** This plan is executed as three Conclave lanes (F, U, P) with
> disjoint file boundaries. Each lane's implementer reads: the spec
> (`docs/superpowers/specs/2026-07-05-uishot-real-pixels-design.md`) → this plan →
> their lane's tasks only. Steps use checkbox (`- [ ]`) syntax for tracking.
> owner: bfb737ff (Detoro) · authority: in-loop

**Goal:** An agent that edits `src/` UI can render the real app headlessly with fixture
data, screenshot it, and see the pixels (plus runtime errors) before claiming done; the
Design view stops forgetting the human's current screen on reload.

**Architecture:** A DEV-only fixture backend behind the single IPC seam
(`src/ipc/commands.ts` `call()` / `src/ipc/events.ts` `useEvent()`) lets the full React
app run in plain Chrome with deterministic data. A `uishot` CLI (port of Arta's
puppeteer-core capture core) drives `http://localhost:1420/?fixture=<scenario>#view=<id>`,
waits for a readiness sentinel, and writes PNGs to `.shots/`. The design-host Shell
persists its active screen in the URL hash + localStorage.

**Tech Stack:** React 19, TypeScript 5.8, Vite 7 (port 1420, strictPort), pnpm,
puppeteer-core (new root devDependency — the ONLY new dependency), Node ≥20.

## Global Constraints

- Package manager is **pnpm**. Never npm/yarn.
- Fixture code activates ONLY when `import.meta.env.DEV && ?fixture=` is present. The
  prod bundle must not grow: the fixture module is reached via dynamic import inside a
  `if (import.meta.env.DEV)` guard, which Vite eliminates in prod builds.
- Fixture data uses **fixed literal timestamps** (base `2026-07-05T12:00:00Z`). No
  `Date.now()`, no `Math.random()` anywhere in `src/fixtures/` — screenshots must be
  byte-reproducible.
- All UI copy and code comments in English (workspace rule).
- Shared checkout: commit ONLY via `conclave stage commit <ws> <slug> -m <msg>` (scoped
  to your lane's boundary). Never raw `git add`/`git commit`.
- Gate evidence must be RECORDED: `conclave task gate <ws> <slug> -- <cmd>` after the
  commit it certifies. A narrated pass does not count.
- TypeScript gate for anything under `src/`: `pnpm build` (runs `tsc && vite build`).
- Interface contracts between lanes (exact strings, do not vary):
  - App URL: `http://localhost:1420/?fixture=<scenario>#view=<viewId>`
  - Scenarios v1: `default`, `empty`
  - View ids v1: `home`, `laneboard`, `memory`, `artifacts`, `blackboard`, `chat`,
    `library`, `builder`, `settings` (`home` = default workspace pane; `design` is
    OUT of scope v1 — it needs the design-host sidecar)
  - Readiness sentinel: `body[data-conclave-ready="1"]`, set only in fixture mode
  - Shots land in `.shots/` (gitignored)

---

## Lane F — Fixture mode (boundary: `src/fixtures/**`, `src/ipc/commands.ts`, `src/ipc/events.ts`, `src/components/AppShell.tsx`)

### Task F1: Fixture mode detection + call seam

**Files:**
- Create: `src/fixtures/mode.ts`
- Create: `src/fixtures/backend.ts`
- Modify: `src/ipc/commands.ts:353-360` (the `call()` function only)

**Interfaces:**
- Produces: `fixtureScenario(): string | null` (mode.ts); `maybeFixtureCall(cmd,
  payload): Promise<{hit: boolean; value?: unknown}>` (backend.ts);
  `FixtureHandlers` type = `{ [K in keyof Commands]?: (req: Commands[K]["req"]) =>
  Commands[K]["res"] | Promise<Commands[K]["res"]> }`. Task F3 registers scenario
  handler maps of this type; Task F4 and Lane U rely on the `?fixture=` query contract.

- [ ] **Step 1: Write `src/fixtures/mode.ts`**

```ts
/** Fixture mode: DEV-only, opted into per page-load via `?fixture=<scenario>`.
 *  Returns the scenario name, or null when the app should talk to the real
 *  Tauri host. Never true in a production build. */
export function fixtureScenario(): string | null {
  if (!import.meta.env.DEV) return null;
  const v = new URLSearchParams(window.location.search).get("fixture");
  return v && v.length > 0 ? v : null;
}
```

- [ ] **Step 2: Write `src/fixtures/backend.ts`**

```ts
import type { Commands } from "../ipc/commands";
import { fixtureScenario } from "./mode";

export type FixtureHandlers = {
  [K in keyof Commands]?: (
    req: Commands[K]["req"],
  ) => Commands[K]["res"] | Promise<Commands[K]["res"]>;
};

// Scenario registry. Task F3 fills these modules in; keeping the imports lazy
// means the datasets load only on first fixture call.
const SCENARIOS: Record<string, () => Promise<{ handlers: FixtureHandlers }>> = {
  default: () => import("./scenarios/default"),
  empty: () => import("./scenarios/empty"),
};

/** Route an IPC call to the active scenario. `hit:false` means "not in fixture
 *  mode — caller should invoke the real host". An ACTIVE scenario with a
 *  missing handler THROWS (loudly visible in the page + uishot stderr), never
 *  silently falls through to Tauri. */
export async function maybeFixtureCall(
  cmd: keyof Commands,
  payload: unknown,
): Promise<{ hit: boolean; value?: unknown }> {
  const scenario = fixtureScenario();
  if (!scenario) return { hit: false };
  const load = SCENARIOS[scenario];
  if (!load) throw new Error(`[fixture] unknown scenario "${scenario}"`);
  const { handlers } = await load();
  const handler = handlers[cmd];
  if (!handler) throw new Error(`[fixture] no handler for command "${cmd}" in scenario "${scenario}"`);
  return { hit: true, value: await handler(payload as never) };
}
```

- [ ] **Step 3: Thread the seam through `call()` in `src/ipc/commands.ts`**

Replace the existing body (keep the surrounding comment about the `null` sentinel):

```ts
export async function call<K extends keyof Commands>(
  ...[cmd, payload]: CallArgs<K>
): Promise<Commands[K]["res"]> {
  // `null` (not `{}`) is the Rust-compatible sentinel for void-req commands:
  // serde deserializes a unit / Value::Null from JSON `null`, never from `{}`.
  const safePayload: unknown = payload ?? null;
  if (import.meta.env.DEV) {
    // Dynamic import inside the DEV guard: prod builds drop this branch and
    // never bundle src/fixtures/*.
    const { maybeFixtureCall } = await import("../fixtures/backend");
    const routed = await maybeFixtureCall(cmd, safePayload);
    if (routed.hit) return routed.value as Commands[K]["res"];
  }
  return invoke<Commands[K]["res"]>("ipc", { cmd, payload: safePayload });
}
```

- [ ] **Step 4: Create placeholder scenario modules so tsc passes**

`src/fixtures/scenarios/default.ts` and `src/fixtures/scenarios/empty.ts`, each
temporarily:

```ts
import type { FixtureHandlers } from "../backend";
export const handlers: FixtureHandlers = {};
```

- [ ] **Step 5: Verify** — `pnpm build` → PASS (tsc + vite). Then `pnpm dev`, open
  `http://localhost:1420/?fixture=default` in a browser: the app must show a crashed
  boot with `[fixture] no handler for command "workspace.list"` (loud-failure proof),
  while plain `http://localhost:1420/` still boots against Tauri-less DEV exactly as
  before (invoke rejections, unchanged behavior).

- [ ] **Step 6: Commit** — `conclave stage commit <ws> uishot-fixture-mode -m "feat(fixture): DEV-only fixture seam behind ipc call()"`

### Task F2: Event seam

**Files:**
- Create: `src/fixtures/events.ts`
- Modify: `src/ipc/events.ts` (the `useEvent` function only, ~line 155)

**Interfaces:**
- Produces: `fixtureListen<T>(event, cb): () => void` and
  `emitFixtureEvent(event, payload)` on a module-level `EventTarget`. v1 scenarios do
  not emit; the seam exists so subscriptions are clean no-ops (no console.error spam)
  and later scenarios can script events.

- [ ] **Step 1: Write `src/fixtures/events.ts`**

```ts
// Local event bus standing in for Tauri's event system in fixture mode.
// v1 scenarios never emit; this exists so useEvent() subscriptions are silent
// no-ops instead of DEV console errors, and so a later scenario CAN emit.
const bus = new EventTarget();

export function fixtureListen<T>(event: string, cb: (payload: T) => void): () => void {
  const h = (e: Event) => cb((e as CustomEvent<T>).detail);
  bus.addEventListener(event, h);
  return () => bus.removeEventListener(event, h);
}

export function emitFixtureEvent<T>(event: string, payload: T): void {
  bus.dispatchEvent(new CustomEvent(event, { detail: payload }));
}
```

- [ ] **Step 2: Branch inside `useEvent`'s subscribe effect** (`src/ipc/events.ts`) —
  at the top of the second `useEffect`, before the `listen<T>(...)` call:

```ts
useEffect(() => {
  let active = true;
  let unlistenFn: UnlistenFn | undefined;

  if (import.meta.env.DEV) {
    // Fixture mode: subscribe to the local bus instead of the Tauri host.
    // Checked inside the effect (not module scope) so HMR picks up URL changes.
    void import("../fixtures/mode").then(async ({ fixtureScenario }) => {
      if (!fixtureScenario() || !active) return;
      const { fixtureListen } = await import("../fixtures/events");
      const off = fixtureListen<T>(event, (p) => {
        if (active) handlerRef.current(p);
      });
      unlistenFn = off as UnlistenFn;
    });
  }
  // ... existing listen<T>(event, ...) chain, wrapped so it only runs when
  // NOT in fixture mode: guard it with a synchronous check —
  // `const inFixture = import.meta.env.DEV && new URLSearchParams(window.location.search).has("fixture");`
  // `if (!inFixture) { listen<T>(...)... }`
  return () => { active = false; unlistenFn?.(); };
}, [event]);
```

Implementation note: prefer the synchronous `inFixture` boolean (shown in the comment)
for BOTH branches — it avoids racing the dynamic import against `listen()`. The
dynamic imports stay inside the `inFixture` branch.

- [ ] **Step 3: Verify** — `pnpm build` → PASS. In the browser at
  `?fixture=default` (still crashing on F1's missing handlers) the console shows NO
  `useEvent: failed to subscribe` lines.

- [ ] **Step 4: Commit** — `conclave stage commit <ws> uishot-fixture-mode -m "feat(fixture): local event bus replaces tauri listen in fixture mode"`

### Task F3: Scenario datasets (`default`, `empty`)

**Files:**
- Create: `src/fixtures/scenarios/data.ts` (shared entities for `default`)
- Modify: `src/fixtures/scenarios/default.ts`, `src/fixtures/scenarios/empty.ts`

**Interfaces:**
- Consumes: `FixtureHandlers` from F1; domain types from `src/ipc/types.ts`.
- Produces: handler coverage for every command the v1 views invoke.

- [ ] **Step 1: Author the `default` dataset.** Fixed constants, realistic content —
  one workspace ("codeup"), 4–6 agents with distinct roles/levels (mirror the real
  roster shape: lead/reviewer/implementer/designer), 2 chat threads with 6–10
  messages, 5 tasks across states, 4 blackboard keys, 3 artifacts, a small memory
  graph (8 nodes / 10 edges). Timestamps: derive by literal offsets from
  `const T0 = "2026-07-05T12:00:00.000Z"`. Skeleton:

```ts
import type { Workspace, WorkspaceAgent, TaskListRow /* … */ } from "../../ipc/types";

export const WS_ID = "fx-ws-codeup";

export const workspaces: Workspace[] = [
  {
    id: WS_ID,
    name: "codeup",
    folderPath: "/Users/dev/code/codeup",
    color: "#7c6af2",
    createdAt: "2026-07-01T09:00:00.000Z",
    // …every field the Workspace type requires, literal values; the compiler
    // is the completeness check — do NOT cast or use `as Workspace`.
  },
];
// agents, tasks, messages, blackboard, artifacts, memoryGraph follow the same
// pattern: fully-typed literals, no casts.
```

- [ ] **Step 2: Wire handlers in `default.ts`** — start with the boot set, e.g.:

```ts
import type { FixtureHandlers } from "../backend";
import { workspaces, agents, tasks /* … */ } from "./data";

export const handlers: FixtureHandlers = {
  "workspace.list": () => workspaces,
  "workspace.use": () => undefined,
  "agentDef.list": () => agentDefs,
  "instance.list": ({ workspaceId }) => agents.filter((a) => a.workspaceId === workspaceId),
  "task.list": ({ workspaceId }) => tasks.filter((t) => t.workspaceId === workspaceId),
  "blackboard.list": () => blackboardEntries,
  "artifact.list": () => artifacts,
  "memory.graph": () => memoryGraph,
  "provider.list": () => providers,
  "tool.list": () => tools,
  "role.list": () => roles,
  "skill.list": () => skills,
  // …grown by the discovery loop in Step 3.
};
```

(Exact request/response field names come from the `Commands` map in
`src/ipc/commands.ts` — the compiler enforces them; the list above names the known
boot+view commands, discovery adds the rest.)

- [ ] **Step 3: Discovery loop until clean.** For each view id in the v1 contract
  (`home`, `laneboard`, `memory`, `artifacts`, `blackboard`, `chat`, `library`,
  `builder`, `settings`): open `http://localhost:1420/?fixture=default#view=<id>`
  (after F4 lands the hash wiring; until then navigate manually), watch the console,
  and add a handler for every `[fixture] no handler for command "X"` until every view
  renders with NO fixture errors. Sessions/PTY surfaces may render empty frames —
  that is the accepted v1 limitation, not an error to chase.

- [ ] **Step 4: `empty.ts`** — same handler keys, empty-state data: zero workspaces
  (or one empty workspace if the shell hard-requires a selection to render anything —
  decide by looking at what `home` renders with `[]`; record the choice in a comment).

- [ ] **Step 5: Verify** — `pnpm build` → PASS; both scenarios render all nine views
  with zero `[fixture]` errors in console.

- [ ] **Step 6: Commit** — `conclave stage commit <ws> uishot-fixture-mode -m "feat(fixture): default + empty scenario datasets"`

### Task F4: Hash-view routing + readiness sentinel (AppShell)

**Files:**
- Modify: `src/components/AppShell.tsx`

**Interfaces:**
- Consumes: `fixtureScenario()` from F1.
- Produces: the `#view=<viewId>` contract and `body[data-conclave-ready="1"]` sentinel
  that Lane U's capture waits on.

- [ ] **Step 1: Add fixture boot effect** after the existing workspace-list boot
  effect (~`AppShell.tsx:89`):

```tsx
// Fixture mode (DEV-only): route the initial view from the URL hash and set a
// readiness sentinel once data + first paint have landed, so a headless
// capture (scripts/uishot.mjs) knows when to shoot. No-op outside ?fixture=.
useEffect(() => {
  if (!import.meta.env.DEV) return;
  void import("../fixtures/mode").then(({ fixtureScenario }) => {
    if (!fixtureScenario()) return;
    const view = /view=([a-z-]+)/.exec(window.location.hash)?.[1] ?? "home";
    const open: Record<string, () => void> = {
      home: () => {},
      laneboard: () => setShowLaneBoard(true),
      memory: () => setShowMemory(true),
      artifacts: () => setShowArtifacts(true),
      blackboard: () => setShowBlackboard(true),
      chat: () => setShowChat(true),
      library: () => setShowLibrary(true),
      builder: () => setShowBuilder(true),
      settings: () => setShowSettings(true),
    };
    (open[view] ?? open.home)();
  });
}, []);
```

- [ ] **Step 2: Auto-select the first workspace in fixture mode.** In the existing
  boot effect that fetches `workspace.list`, after `setWorkspaces(ws)`: if fixture
  mode is active and `ws.length > 0`, also `setActiveWorkspaceId(ws[0].id)` (reuse
  whatever the click-path setter does — match the existing activation code path so
  dependent fetches fire).

- [ ] **Step 3: Set the sentinel** — add state `const [booted, setBooted] = useState(false)`
  set to `true` at the end of the successful boot fetch, then:

```tsx
useEffect(() => {
  if (!import.meta.env.DEV || !booted) return;
  void import("../fixtures/mode").then(({ fixtureScenario }) => {
    if (!fixtureScenario()) return;
    // Double-rAF: sentinel lands after the routed view's first real paint.
    requestAnimationFrame(() =>
      requestAnimationFrame(() => {
        document.body.dataset.conclaveReady = "1";
      }),
    );
  });
}, [booted]);
```

- [ ] **Step 4: Verify** — `pnpm build` → PASS. In the browser:
  `?fixture=default#view=laneboard` opens straight onto the Lane Board;
  `document.body.dataset.conclaveReady` is `"1"`; plain `http://localhost:1420/`
  never sets the attribute.

- [ ] **Step 5: Commit, move task to review** —
  `conclave stage commit <ws> uishot-fixture-mode -m "feat(fixture): hash-view routing + data-conclave-ready sentinel"`,
  then `conclave task gate <ws> uishot-fixture-mode -- sh -c "pnpm build"`, then
  state → review with a READY note listing the nine verified views.

---

## Lane U — uishot CLI (boundary: `scripts/uishot.mjs`, `scripts/uishot-eval.mjs`, `package.json`, `pnpm-lock.yaml`, `.gitignore`)

### Task U1: Capture CLI

**Files:**
- Create: `scripts/uishot.mjs`
- Modify: `package.json` (add script + devDependency), `.gitignore` (add `.shots/`)

**Interfaces:**
- Consumes: Lane F's URL + sentinel contract (Global Constraints block). Buildable in
  parallel with Lane F; the live end-to-end run needs F merged.
- Produces: `pnpm uishot <viewId> [--scenario default|empty] [--full] [--out <path>]
  [--viewport WxH]` → writes `.shots/<viewId>-<scenario>.png`, exit 0 on success,
  exit 1 on pageerror/missing sentinel/no Chrome. Console errors from the page are
  forwarded to stdout prefixed `[page]`.

- [ ] **Step 1:** `pnpm add -D puppeteer-core` (root). Add to `package.json` scripts:
  `"uishot": "node scripts/uishot.mjs"`. Append `.shots/` to `.gitignore`.

- [ ] **Step 2: Write `scripts/uishot.mjs`.** Port of
  `/Users/detoro/code/arta/vite/headless-snapshot.ts` (findChrome table, single-browser
  reuse, `--no-sandbox --hide-scrollbars --force-color-profile=srgb --disable-gpu`,
  `deviceScaleFactor: 2`, UNCLAMP string for `--full`) with these swaps:

```js
#!/usr/bin/env node
// uishot — screenshot the REAL src/ app (fixture mode) so an agent can see the
// pixels it just changed. Port of arta's headless-snapshot core; see
// docs/superpowers/specs/2026-07-05-uishot-real-pixels-design.md.
import fs from "node:fs";
import path from "node:path";
import { spawn } from "node:child_process";

const BASE = "http://localhost:1420";
const SENTINEL = 'body[data-conclave-ready="1"]';

// --- arg parsing (no deps) ---
const args = process.argv.slice(2);
const view = args.find((a) => !a.startsWith("--"));
if (!view) { console.error("usage: pnpm uishot <viewId> [--scenario default|empty] [--full] [--out p] [--viewport WxH]"); process.exit(2); }
const opt = (name, dflt) => { const i = args.indexOf(`--${name}`); return i >= 0 ? args[i + 1] : dflt; };
const scenario = opt("scenario", "default");
const full = args.includes("--full");
const [vw, vh] = (opt("viewport", "1440x900")).split("x").map(Number);
const out = opt("out", path.join(".shots", `${view}-${scenario}.png`));

// --- findChrome(): copy arta's per-platform table + CHROME_PATH/PUPPETEER_EXECUTABLE_PATH env override verbatim ---
// --- ensureServer(): fetch(BASE, 1s timeout); on failure spawn("pnpm",["dev"],{detached:true, stdio:"ignore"}).unref(), then poll BASE every 500ms for up to 30s; print "[uishot] reusing dev server" vs "[uishot] started dev server (pnpm dev)" ---
// --- capture ---
const puppeteer = (await import("puppeteer-core")).default;
const executablePath = findChrome();
if (!executablePath) { console.error("[uishot] no Chrome found — install Chrome or set CHROME_PATH"); process.exit(1); }
await ensureServer();
const browser = await puppeteer.launch({ executablePath, headless: true,
  args: ["--no-sandbox", "--hide-scrollbars", "--force-color-profile=srgb", "--disable-gpu"] });
let failed = false;
try {
  const page = await browser.newPage();
  await page.setViewport({ width: vw, height: vh, deviceScaleFactor: 2 });
  page.on("console", (m) => { if (m.type() === "error") console.log(`[page] console.error: ${m.text()}`); });
  page.on("pageerror", (e) => { failed = true; console.log(`[page] pageerror: ${e.message}`); });
  const url = `${BASE}/?fixture=${encodeURIComponent(scenario)}#view=${encodeURIComponent(view)}`;
  await page.goto(url, { waitUntil: "load", timeout: 20000 });
  const ready = await page.waitForSelector(SENTINEL, { timeout: 20000 }).then(() => true).catch(() => false);
  if (!ready) { failed = true; console.log("[uishot] readiness sentinel never appeared (page crashed or fixture handler missing?)"); }
  await new Promise((r) => setTimeout(r, 300));
  fs.mkdirSync(path.dirname(out), { recursive: true });
  if (full) { await page.evaluate(UNCLAMP_IN_PAGE); await new Promise((r) => setTimeout(r, 60)); }
  fs.writeFileSync(out, await page.screenshot({ type: "png", fullPage: full }));
  console.log(`[uishot] wrote ${out} (${vw}x${vh}@2x, scenario=${scenario})`);
} finally { await browser.close(); }
process.exit(failed ? 1 : 0);
```

The two `// ---` blocks marked findChrome/ensureServer are written out in full in the
implementation (findChrome is a verbatim copy of arta's table; ensureServer is the
fetch-poll-spawn described inline). `UNCLAMP_IN_PAGE` is a verbatim copy of arta's
string (headless-snapshot.ts:101-114). A shot is still WRITTEN on failure when
possible — a broken screenshot is evidence, the exit code carries the verdict.

- [ ] **Step 3: Verify offline behavior** — `pnpm uishot home` on a machine with
  Chrome but Lane F not yet merged: expect exit 1 with the sentinel message and a PNG
  of the crashed page in `.shots/home-default.png`. That failure mode IS the loud
  fixture-missing contract working.

- [ ] **Step 4: Commit** — `conclave stage commit <ws> uishot-cli -m "feat(uishot): headless real-app screenshot CLI (puppeteer-core)"`

### Task U2: Smoke eval (post-integration gate)

**Files:**
- Create: `scripts/uishot-eval.mjs`

**Interfaces:**
- Consumes: `pnpm uishot` from U1; Lane F merged.
- Produces: the repeatable gate command for UI lanes and for this plan's acceptance:
  `node scripts/uishot-eval.mjs`.

- [ ] **Step 1: Write `scripts/uishot-eval.mjs`** — runs
  `pnpm uishot <view> --scenario <s>` for the matrix
  `[home, laneboard] × [default, empty]` via `child_process.spawnSync`, then asserts
  for each: exit code 0, output PNG exists, size > 10 KB, first 8 bytes equal the PNG
  magic (`89 50 4E 47 0D 0A 1A 0A`). Prints one PASS/FAIL line per cell and exits
  non-zero on any FAIL.

- [ ] **Step 2: Verify** — with Lane F merged: `node scripts/uishot-eval.mjs` → 4×
  PASS. Record it: `conclave task gate <ws> uishot-cli -- node scripts/uishot-eval.mjs`.

- [ ] **Step 3: Commit, move to review** — `conclave stage commit <ws> uishot-cli -m "feat(uishot): smoke eval matrix"`; READY note includes the `.shots/*.png` paths AND
  confirmation you OPENED and looked at each PNG (this lane eats its own dogfood).

---

## Lane P — Design view screen persistence (boundary: `design-host/src/Shell.tsx`, `design-host/src/screenSelection.ts`, `design-host/test/screen-selection.test.mjs`)

### Task P1: Persist + restore the active screen

**Files:**
- Create: `design-host/src/screenSelection.ts`
- Create: `design-host/test/screen-selection.test.mjs`
- Modify: `design-host/src/Shell.tsx:14-34`

**Interfaces:**
- Produces: hash format `#/<screenId>`; localStorage key
  `conclave-design-active:<projectId>`. Pure precedence helper:
  `pickInitialScreen(hashScreen: string | null, stored: string | null, ids: string[]): string | null`.

- [ ] **Step 1: Write the pure helper** `design-host/src/screenSelection.ts`:

```ts
// Selection restore precedence for the design canvas: URL hash (survives
// reload in-place) beats localStorage (survives tab close), beats default.
export function pickInitialScreen(
  hashScreen: string | null,
  stored: string | null,
  ids: string[],
): string | null {
  if (hashScreen && ids.includes(hashScreen)) return hashScreen;
  if (stored && ids.includes(stored)) return stored;
  return ids.find((id) => id === "welcome") ?? ids[0] ?? null;
}

export function parseHashScreen(hash: string): string | null {
  const m = /^#\/(.+)$/.exec(hash);
  return m ? decodeURIComponent(m[1]) : null;
}
```

- [ ] **Step 2: Write the failing test** `design-host/test/screen-selection.test.mjs`
  (node's built-in runner; load the TS via esbuild like `review/grade.mjs` does):

```js
import { test } from "node:test";
import assert from "node:assert/strict";
import { transformSync } from "esbuild";
import fs from "node:fs";
import { fileURLToPath } from "node:url";

const src = fs.readFileSync(new URL("../src/screenSelection.ts", import.meta.url), "utf8");
const mod = await import(
  "data:text/javascript," + encodeURIComponent(transformSync(src, { loader: "ts" }).code)
);
const { pickInitialScreen, parseHashScreen } = mod;

test("hash wins over stored", () => {
  assert.equal(pickInitialScreen("b", "c", ["a", "b", "c"]), "b");
});
test("stored wins over default when hash invalid", () => {
  assert.equal(pickInitialScreen("gone", "c", ["a", "b", "c"]), "c");
});
test("falls back to welcome, then first, then null", () => {
  assert.equal(pickInitialScreen(null, null, ["x", "welcome"]), "welcome");
  assert.equal(pickInitialScreen(null, null, ["x", "y"]), "x");
  assert.equal(pickInitialScreen(null, null, []), null);
});
test("parseHashScreen", () => {
  assert.equal(parseHashScreen("#/lane-board"), "lane-board");
  assert.equal(parseHashScreen(""), null);
  assert.equal(parseHashScreen("#other"), null);
});
```

- [ ] **Step 3: Run to fail-then-pass** —
  `cd design-host && node --test test/screen-selection.test.mjs` (fails only if the
  helper is wrong; write test and helper in the same change, verify all green).

- [ ] **Step 4: Wire into `Shell.tsx`** — replace lines 14-20 area:

```tsx
import { pickInitialScreen, parseHashScreen } from "./screenSelection";

const PROJECT = new URLSearchParams(window.location.search).get("project") ?? "";
const LS_KEY = `conclave-design-active:${PROJECT}`;

export function Shell({ screens, screenIds }: ShellProps) {
  const [active, setActiveState] = useState<string | null>(() =>
    pickInitialScreen(
      parseHashScreen(window.location.hash),
      localStorage.getItem(LS_KEY),
      screenIds,
    ),
  );

  // Persist every selection: hash for reload-in-place, localStorage for later.
  const setActive = (id: string | null) => {
    setActiveState(id);
    if (id) {
      history.replaceState(null, "", `#/${encodeURIComponent(id)}`);
      localStorage.setItem(LS_KEY, id);
    }
  };

  useEffect(() => {
    if (active && !screenIds.includes(active)) setActive(pickInitialScreen(null, null, screenIds));
    else if (!active && screenIds.length) setActive(pickInitialScreen(null, null, screenIds));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [screenIds, active]);
  // …rest unchanged: Switcher onSelect={setActive}, ScreenHost key={active}.
```

- [ ] **Step 5: Verify live** — with the design-host running (Conclave Design view or
  `node design-host/bin/host.mjs`): select a non-default screen, touch that screen's
  file to force a reload → the SAME screen is showing afterward; hard-refresh the
  iframe → same screen; delete the screen file → falls back to default without crash.

- [ ] **Step 6: Commit, gate, review** —
  `conclave stage commit <ws> design-shell-remember -m "fix(design-host): remember active screen across reloads (hash + localStorage)"`;
  `conclave task gate <ws> design-shell-remember -- sh -c "cd design-host && node --test test/screen-selection.test.mjs"`;
  state → review with a READY note describing the live reload check.

---

## Integration & policy (lead-owned, after lanes merge)

1. Merge order: F → U (U2's eval gate re-run at integration), P independent.
2. Re-run gates at the integrated HEAD: `pnpm build`, `node scripts/uishot-eval.mjs`,
   design-host node --test.
3. Amend the implementer skill sidecars + workspace protocol: UI lanes touching
   `src/` MUST run uishot on every affected view, LOOK at the PNG, and attach shot
   paths to the READY note; gate recorded via `conclave task gate`. Known caveat to
   state verbatim: PTY/terminal surfaces render empty in fixture mode — do not chase
   phantom terminal bugs from shots.
4. Update the r13 policy record (ruling 35968ae3): "env-blocked" rationale superseded
   for fixture-reachable UI; human checklist remains final acceptance.
5. Save memory entries: fixture-mode seam location, uishot usage, the sentinel
   contract.

## Risk ledger (implementers: read before starting)

- **Stale dev server reuse:** Vite 1420 is strictPort; uishot reuses a running server
  which may serve a peer's older working tree in the shared checkout. uishot prints
  reuse-vs-started; when in doubt, kill the server and let uishot restart it.
- **PTY/xterm views:** render empty frames in fixture mode — accepted v1.
- **`useEvent` double-subscribe:** the fixture branch must be exclusive with the Tauri
  branch (synchronous `inFixture` check), or events fire twice in dev.
- **Fixture literals drift:** never use `as`-casts in `src/fixtures/` — the compiler
  is the drift alarm.
- **`empty` scenario shell requirement:** the shell may require ≥1 workspace to render
  anything; resolve per F3 Step 4 and record the decision in a code comment.
- **StrictMode double-effects:** AppShell effects run twice in dev; the sentinel and
  view-routing effects are idempotent as written — keep them so.
