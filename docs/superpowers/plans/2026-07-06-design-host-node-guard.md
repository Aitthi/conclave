# design-host-node-guard — survive broken pnpm shims and pre-Node-20 machines

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
implementer: Dew (40d90aed-bdfe-4dfb-837b-1daa22d796b1)
escalation: design/spec conflicts → Detoro (task challenge); implementation judgment within this plan → Dew, logged as task notes.
Follow-up to `2026-07-06-design-host-packaging.md` (merged 810088f).

## Mission

Second field failure on the target machine (user `Tharadon`, screenshot
2026-07-06 10:50): the bundled sidecar now resolves (previous fix works), but
`pnpm install` crashes INSIDE pnpm itself:

```
pnpm install failed: /Users/Tharadon/.cache/node/corepack/v1/pnpm/11.5.2/bin/pnpm.cjs:3
import('./pnpm.mjs') ^ TypeError: Invalid host defined options ... Node.js v18.20.8
```

Two distinct defects:

1. **D4's `command -v pnpm` check proved insufficient** (AMENDMENT to the
   packaging plan's D4): a corepack shim EXISTS on PATH but the pnpm it
   launches (11.5.2) cannot run on that machine's node (v18.20.8). Presence
   is not workability — the fallback to npm never fired.
2. **No minimum-node guard**: `design-host` deps require Node ≥ 20 at runtime
   (`react-router-dom` engines `>=20.0.0`; verified in
   `design-host/node_modules` on 2026-07-06 — vite allows ^18 but the dep
   tree does not). Node 18 is also past EOL. Even with npm succeeding, a
   Node-18 machine would fail later and more confusingly. Fail fast, at the
   door, with a message that names the found version, its path, and the fix.

## Decisions (settled)

- **G1 — Workability, not presence.** In `ensure_deps_installed`, replace
  `command -v pnpm` with running `pnpm --version` through the login shell;
  choose pnpm only when that exits 0 with a parseable version on stdout.
  Anything else (missing, corepack shim crash, non-zero) → npm. Log which
  manager was chosen and why (one `eprintln!` line, existing convention).
- **G2 — Minimum Node 20, enforced in `resolve_node()`.** After resolving the
  node path, run `<node> --version`; parse `vMAJOR.MINOR.PATCH`. If
  major < 20, return a new `DesignHostError` variant (e.g.
  `NodeTooOld { found: String, path: PathBuf }`) whose Display reads like:
  `design view requires Node.js 20 or newer; found v18.20.8 at
  /Users/…/.nvm/versions/node/v18.20.8/bin/node — install a newer node
  (e.g. \`nvm install 20\` or \`brew install node\`) and relaunch`.
  Constant `MIN_NODE_MAJOR: u32 = 20` with a comment citing
  react-router-dom's engines field as the driver — update the comment if the
  dep tree's floor ever moves.
  `resolve_node` is shared by `ensure_running()` and `review()`, so both
  surfaces get the guard for free. If `--version` itself fails to run or
  parse, treat as `NodeNotFound`-grade failure with the raw output in the
  message — do NOT silently proceed.
- **G3 — No behavior change on healthy machines.** pnpm working + node ≥ 20
  (this dev machine: pnpm ok, node v22.23.1) must take exactly the same path
  as today. Dev mode untouched.

## Steps

1. `src-tauri/src/engine/runtime/design_host.rs` only (boundary unchanged
   from the packaging lane):
   - G1 in `ensure_deps_installed`.
   - G2: version check + parse helper (`fn parse_node_major(v: &str) ->
     Option<u32>` or similar) + new error variant + Display arm.
2. Unit tests: version-string parsing (`"v18.20.8"` → 18, `"v22.23.1"` → 22,
   garbage → None); Display text of `NodeTooOld` names version, path, and a
   remedy. Keep manager-choice logic in a shape where the pnpm-broken → npm
   decision is a testable pure function if reasonably cheap; otherwise cover
   by construction and say so in the READY note.
3. Gates (commit first):
   - `cargo test --manifest-path src-tauri/Cargo.toml design_host`
   - `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings` is
     NOT in this repo's gate set — skip unless already standard; do not add
     new gate kinds in this lane.
4. Smoke on this machine (READY note): trigger the design view once via
   `pnpm tauri dev` or the existing built app — confirm the happy path still
   boots (G3). A broken-pnpm simulation is NOT required; unit tests carry G1.

## Risk ledger

- The `-l -i` login shell can print rc-file noise on stdout; `pnpm --version`
  parsing must tolerate leading junk (take the LAST line that parses as a
  version, or match a `\d+\.\d+\.\d+` line) — nvm hooks love to chat.
- `node --version` output is stable (`vX.Y.Z\n`) but guard the parse anyway.
- Do NOT try to auto-fix the user's pnpm (no corepack enable/prepare, no
  global installs) — we only choose between what already works.
- After this lands, the Tharadon machine will show the clear NodeTooOld error
  until node ≥ 20 is installed there. That is the intended behavior, not a
  bug report waiting to happen — the human knows.

## Out of scope

- Bundling a node runtime (unchanged D5 of the packaging plan).
- Auto-upgrading/repairing the target machine's toolchain.
