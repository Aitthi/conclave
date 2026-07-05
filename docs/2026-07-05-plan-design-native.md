# Plan: Conclave-native Design view (feature `design-native`)

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro, lead) · authority: in-loop
requested by the human 2026-07-05 ~16:02 (screenshot + follow-ups), forks settled by human ruling same hour.

## What this is

Replace the Arta-embedded Design view with Conclave's OWN design system: agents
write React screens into a workspace-owned folder, the engine serves them live
through our own Vite canvas host, and the app shows that canvas **full-window**
(no Rail, no Roster) with the agent terminal kept on the right.

## Human-ruled decisions (final — do not re-open)

- **D1 — Fully separate from Arta.** The new system never reads or writes
  `.arta/**`. REJECTED: new UI over `.arta` data (human: "แยกขาดทั้งระบบ ไม่แตะ
  .arta เลย"); keep-Arta-embed-but-fullscreen.
- **D2 — Format: TSX screens + our own dev server.** Designs are React screens
  under `<workspaceFolder>/design/screens/*.tsx`, rendered live (HMR) by a
  Conclave-owned Vite host. REJECTED: HTML-in-sandboxed-iframe (no HMR, no
  composable React).
- **D3 — Full-window layout.** While the Design view is open, the Rail
  (workspaces column) and Roster (agent list) are hidden; the window is
  canvas-left + agent-terminal-right. The close X restores the normal layout.
- **D4 — Replace, don't coexist.** The Rail Design button opens the new view.
  The vendored Arta viewer (`design-viewer/`) is removed from the app; anyone
  wanting the Arta studio opens it in a browser via the arta plugin as before.
  REJECTED: two design entries side by side.

## Lead rulings (recorded here; challenge via `task challenge` if wrong)

- **D5 — Reuse the supervisor machinery.** The new host is supervised exactly
  like the old sidecar (`runtime/design_viewer.rs` pattern): registry
  write-before-spawn, login-shell `node` resolution, first-run `pnpm install`,
  health-check + capped-backoff respawn, single-flight lock. Security config
  carries over verbatim from ruling 4d81be26: bind 127.0.0.1 only, `cors:
  false`, explicit `allowedHosts`, `fs.deny` secrets patterns.
  AMENDED (lead, at Lane A review): the original wording here said
  "`server.fs.allow` scoped" — that was wrong; static `fs.allow` cannot cover
  workspace `design/` dirs resolved only at request time, which is exactly why
  the old posture used `fs.strict: false` and why "registry-driven dynamic
  fs.allow" sat in the (superseded) hardening task. The 4d81be26 posture as
  actually shipped — `fs.strict: false` + the cors/allowedHosts/fs.deny trio,
  residual same-user `/@fs/` read accepted and documented — carries over
  verbatim; the Host/Origin guard middleware stays the deferred follow-up (D7).
- **D6 — Screen discovery lives in the host, not IPC.** The host discovers
  `screens/*.tsx` itself; the `design.ensure` / `design.status`
  IPC shape (`{ workspaceId } → { url, port, projectId, running }`) is
  UNCHANGED so `DesignView.tsx`'s contract stays stable. No new backend
  commands. AMENDED (lead, at Lane A review): the original wording prescribed
  `import.meta.glob`, which cannot take a runtime-resolved external directory
  (its pattern must be compile-time static, and `design/` dirs live outside
  the host package root). The mechanism is a generated per-project manifest
  (`design-host/vite/screens.ts`), the same technique the predecessor's
  `proto-manifest.ts` used for the identical constraint. The RULE of D6 —
  discovery inside the host, zero new IPC — is unchanged. Credit: Dew for
  flagging instead of silently deviating.
- **D7 — `design-viewer-hardening` task is superseded** (it hardened the Arta
  embed). Its middleware/fs.allow requirements become acceptance criteria of
  Lane A below. Prod packaging of the sidecar (node-at-runtime question)
  stays DEFERRED, unchanged in status — record it as a follow-up, do not
  solve it in this feature.

## Lanes (independent boundaries; lead integrates)

### Lane C — `design-native-canon` (Arta, designer)
Design the new view's chrome on the Arta canvas (the design MEDIUM stays
Arta's own tooling — that is not a D1 violation; D1 is about the PRODUCT
feature's data): full-window frame (canvas-left + terminal-right, no
Rail/Roster), canvas header (workspace name, open-in-browser, close X — the
existing `DesignView.tsx` header is the visual baseline), screen switcher
(placement + look; the list itself renders inside the host iframe per D6, so
design it as an in-canvas floating element), empty state (no `design/` folder
yet / zero screens), error states (node missing, sidecar failed — reuse
current DesignView error card language). Output: proto screens + a canon
commit SHA the view lane will pin.

### Lane A — `design-host` (engine implementer)
1. **New vendored host** at `design-host/`: minimal Vite + React app —
   `import.meta.glob('/screens/**/*.tsx')`, screen switcher UI per canon,
   renders the selected screen full-bleed, HMR passthrough. No spec panel, no
   tabs, no Arta anything. Vite config per D5 security lines.
2. **Supervisor**: point the `design_viewer.rs` machinery at `design-host/`
   with project root `<workspaceFolder>/design/` (rename module to
   `design_host.rs` — semantic rename, keep the load-bearing
   registry-write-before-spawn and single-flight lock comments).
3. **Scaffold**: `design.ensure` creates `design/screens/welcome.tsx` +
   `design/lib/` when missing (mirror the old `scaffold_if_missing` shape).
4. **Remove `design-viewer/`** (vendored Arta viewer) and its registry
   entries; `commands/design.rs` keeps its IPC names and response shape (D6).
5. Tests: port the `ensure_round_trip` test to the new root; keep the
   response-shape serialisation pinned.

Boundary: `design-host/**, src-tauri/src/engine/runtime/design_viewer.rs,
src-tauri/src/engine/runtime/design_host.rs, src-tauri/src/engine/runtime/mod.rs,
src-tauri/src/engine/commands/design.rs, design-viewer/**` (the last for
deletion only).

AMENDMENT (lead, post-hoc — plan defect found by Dew): the module rename has
ONE call site outside this boundary, `src-tauri/src/lib.rs` (RunEvent::Exit →
`kill_on_exit`). The original boundary omitted it, so the crate could not
compile from in-boundary commits alone. Dew landed the 3-line mechanical fix
as a separate scoped raw commit (3c7220d) per the standing boundary-widening
convention and flagged it — accepted and ratified by lead ruling on the
design-host ledger. Guard for future plans: when a lane renames a module, grep
the crate for the module path (`codegraph find-refs`) and put every call site
in the boundary up front.

### Lane B — `design-native-view` (frontend; CUT AFTER canon + Lane A land)
1. **AppShell full-window mode**: when `designOpen`, hide the Rail and Roster
   columns (`AppShell.tsx:203-244` area); restore on close. Keep the design
   slot always-mounted contract in `WorkspacePane.tsx:335` (that comment is
   load-bearing — terminal must not remount).
2. **DesignView chrome** per canon: header, error/empty states; iframe src
   from `design.ensure` exactly as today (`DesignView.tsx:134`).
3. Keep canvas-left + terminal-right split (`WorkspacePane.tsx:309-318`),
   now spanning the full window.

Boundary: `src/components/AppShell.tsx, src/components/DesignView.tsx,
src/components/WorkspacePane.tsx` — NO overlap with `position-ui`'s boundary
(Roster/Builder/LaneBoard/Position/positions).

## Global constraints (every lane inherits)

- Shared checkout: `conclave stage commit` only; choke-point semantic-diff
  guard (memory a6a3dd26) applies to `commands/design.rs`, `runtime/mod.rs`.
- App UI copy is English.
- Wiring guard (plan 6c70fe7): if a lane thinks it needs `router.rs`,
  `bus.rs`, `db.rs`, or `src/ipc/*` — STOP, escalate to the lead; D6 says it
  shouldn't.
- Live UI proof is env-blocked for agents → r13 human checklist per ruling
  35968ae3.

## Risk ledger

- The old sidecar registry (`~/Library/Application Support/…` viewer registry)
  may hold stale `design-viewer` entries after the swap — Lane A must handle
  an existing registry file gracefully (upsert semantics already do).
- A workspace whose folder link is missing (`folder_path` NULL) cannot host
  `design/` — DesignView already has an error card for this; keep it.
- `import.meta.glob` picks up new screen FILES only on Vite's file-watcher
  event — verify add-a-file-while-open updates the switcher (it does in Vite
  6 dev mode; test it in the host's own dev run, not just unit tests).
- r12 checklist section [B] tests the OLD embed — superseded when Lane B
  lands; note it on the checklist key at integration, don't delete history.
