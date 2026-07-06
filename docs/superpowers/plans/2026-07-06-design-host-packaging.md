# design-host-packaging — ship the design-host sidecar inside the packaged app

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
implementer: Dew (40d90aed-bdfe-4dfb-837b-1daa22d796b1)
escalation: design/spec conflicts → Detoro (task challenge); implementation judgment within this plan → Dew, logged as task notes.

## Mission

A packaged Conclave.app copied to another machine cannot start the Design view.
Screenshot evidence (human, 2026-07-06): `design host sidecar failed to start:
pnpm install failed: zsh:cd:1: no such file or directory:
/Users/detoro/code/codeup/src-tauri/../design-host`.

Root cause (verified by source trace, all breadcrumbs consistent):

1. `design_host_dir()` (`src-tauri/src/engine/runtime/design_host.rs:287`)
   prefers `Contents/Resources/design-host` next to the exe — but
   `src-tauri/tauri.conf.json:39-43` bundles only `skills`, `roles`, and
   `THIRD-PARTY-NOTICES.md`, so the bundled dir never exists.
2. Fallback is `concat!(env!("CARGO_MANIFEST_DIR"), "/../design-host")`
   (line 293) — a compile-time path of the BUILD machine, absent elsewhere.
3. `ensure_deps_installed` (line 343-363) runs `cd '<missing>' && pnpm install`
   → the exact `zsh:cd:1` error in the screenshot.

This was the D7 deferral in `docs/2026-07-05-plan-design-native.md` ("Prod
packaging of the sidecar stays DEFERRED — record it as a follow-up"). The
follow-up is now due. This plan closes it.

## Decisions (settled — do not re-open; challenge with evidence if wrong)

- **D1 — Bundle the design-host SOURCE, never `node_modules`.** Add per-entry
  mappings to `tauri.conf.json` `bundle.resources` (map form, source paths are
  relative to `src-tauri/`):
  `../design-host/bin → design-host/bin`, and likewise for `src`, `vite`,
  `review`, `index.html`, `package.json`, `pnpm-lock.yaml`, `vite.config.ts`,
  `tsconfig.json`, `tsconfig.app.json`, `tsconfig.node.json`.
  EXCLUDED: `node_modules` (203 MB, pnpm symlink store — unbundleable),
  `evals/`, `test/`, `*.tsbuildinfo`. Before finalizing, grep `bin/`, `vite/`,
  `vite.config.ts`, and `review/` for any runtime import that reaches into an
  excluded dir; if one exists, include that file, don't widen to the dir.
- **D2 — Run from a writable copy, never inside the .app.** When
  `bundled_design_host_dir()` exists, sync it to
  `design_home_dir()?/runtime` (that's
  `~/Library/Application Support/Conclave/design-host/runtime`) and return the
  runtime dir from `design_host_dir()`. Writing `node_modules` into
  `Contents/Resources` would break the code signature, and App Translocation /
  DMG mounts make the bundle read-only anyway. This is load-bearing — do not
  "optimize" it away.
- **D3 — Staleness by content fingerprint.** Fingerprint = SHA-256 (crate
  `sha2`, already in `src-tauri/Cargo.toml:41`) over the bundled tree: walk
  sorted-by-relative-path, hash `relpath bytes + 0x00 + file contents` per
  file. Store as `.bundle-fingerprint` inside the runtime dir. On mismatch or
  absence: delete the runtime dir, copy the tree, write the fingerprint (write
  it LAST, so a killed half-copy re-syncs next run). On match: return
  immediately — no copy, keep the installed `node_modules`.
- **D4 — Package-manager fallback.** `ensure_deps_installed` resolves `pnpm`
  via the login shell (`command -v pnpm`, same pattern as `resolve_node`);
  if absent, fall back to `npm install --no-audit --no-fund` (npm ships with
  node). Error messages must name which manager actually ran. Accepted
  tradeoff: npm ignores `pnpm-lock.yaml`, so versions resolve by semver range
  on pnpm-less machines — recorded, acceptable for now.
- **D5 — `node` on the target machine stays a prerequisite.** `resolve_node`'s
  `NodeNotFound` path is unchanged; bundling a node runtime is out of scope.
- **D6 — Dev mode is untouched.** No bundled dir → the existing
  `CARGO_MANIFEST_DIR/../design-host` fallback, exactly as today. All existing
  behavior on `tauri dev` / `cargo run` must be byte-identical.

## Steps

1. `src-tauri/tauri.conf.json` — add the D1 resource map entries.
2. `src-tauri/src/engine/runtime/design_host.rs` —
   - Make `design_host_dir()` fallible (it now does IO): return
     `Result<PathBuf, DesignHostError>`; sync failures surface as
     `DesignHostError::Spawn(<clear message naming the failing path>)` so they
     land in the existing DesignView error card. Update both callers
     (`ensure_running`'s spawn path and `review()`).
   - Add the D2/D3 sync: `fn sync_bundled_to_runtime(bundled: &Path) ->
     io::Result<PathBuf>` + fingerprint helpers. Keep it std-fs; the tree is
     < 1 MB.
   - D4 fallback in `ensure_deps_installed`.
3. Unit tests (same file, `#[cfg(test)]`, tempdir-based like the module's
   existing tests): fingerprint is stable across two walks; changes when a
   file's bytes change; sync copies the tree and skips when fingerprint
   matches; interrupted copy (missing fingerprint file) re-syncs.
4. Gates, in order (commit first — gates pin HEAD):
   - `cargo test --manifest-path src-tauri/Cargo.toml design_host`
   - `pnpm tauri build` then verify the bundle layout:
     `sh -c 'app=$(ls -d src-tauri/target/release/bundle/macos/*.app | head -1); test -f "$app/Contents/Resources/design-host/bin/host.mjs" && test -f "$app/Contents/Resources/design-host/package.json" && ! test -e "$app/Contents/Resources/design-host/node_modules" && echo BUNDLE-OK'`
     — expected output `BUNDLE-OK`, exit 0.
5. Manual smoke (report in the READY note, with what you observed): launch the
   BUILT app binary (`.../Contents/MacOS/…`), open a workspace's Design view,
   confirm the canvas boots; confirm
   `~/Library/Application Support/Conclave/design-host/runtime` now exists
   with `node_modules` installed there and a `.bundle-fingerprint` file.
   (On this machine the bundled dir exists, so the packaged branch — the one
   that was broken — is exactly what this exercises.)

No UI canon needed: no `src/` change; the existing error card is the only UI
surface and it is untouched. The UI Pixel Gate does not apply.

## Risk ledger

- Tauri v2 `resources` map with `../` sources: the map form controls the
  destination explicitly, but VERIFY the post-build layout matches
  `bundled_design_host_dir()`'s expectation (`Contents/Resources/design-host/…`)
  before trusting it — that's what gate 4b is for. If Tauri nests it
  differently (e.g. an `_up_` segment), fix the map targets, not the Rust.
- `design-host/vite.config.ts` may carry `server.fs.allow` entries tied to the
  host dir; project dirs come from the registry and are already external, but
  confirm the canvas still serves screens when the host runs from app-data.
- First-run install on the target machine takes real wall-clock time; the
  existing `eprintln!` progress lines are the accepted UX (unchanged scope).
- `bundled_design_host_dir()` assumes the macOS .app layout (as does
  `skills_dir()`) — Windows/Linux packaging stays out of scope, same as before.
- The runtime dir is shared by app versions; D3's fingerprint makes an app
  update re-sync automatically. Do not key it by app version — a dev rebuild
  with unchanged design-host must NOT re-install.

## Out of scope

- Bundling a node runtime (D5).
- Prebuilding the sidecar to skip Vite-at-runtime (bigger redesign; the
  sidecar IS a Vite dev server by design — see design-native plan D-series).
- Windows/Linux bundle layouts.
