# ctx-proxy full removal

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro) · authority: in-loop

## Why (decision record)

Human directive 2026-08-16 ("งั้นก็เอา Feature นี้ออก"), after the H1 economics
verdict: every savings hypothesis concluded NO-GO — Phase-1 dedup rewrite
(shelved, plan A9), M2 naive prefix-checkpoint (NO-GO, bb `measure:proxy-m2-q`),
H1/H2 hybrid summarization (measured n_h=14.42 vs bar ≤2, q_h=0.7875,
C_gen=$2.70/sample; bb `plan:hybrid-h1`, `proxy_summary_metric` row 7).
Root economics: cache reads at 0.5 USD/Mtok make raw forwarding cheap; the
one-time full re-read for generation never amortizes. The proxy has no
remaining non-measurement purpose (verified: M1 was a fix internal to the
proxy's own sampling; routing is fail-open; nothing user-facing depends on it).

Council task `infinity-turn-checkpoint` (Phase-2) was abandoned with this
ruling on its ledger. Rejected alternative: keep a passthrough-only proxy as
observability — rejected because every metric consumer is being deleted and a
live axum listener on every agent's request path is pure carrying cost.

## Scope — one lane, one implementer

Remove the entire ctx-proxy subsystem: the loopback listener (port 18787), the
H1/H2 shadow lanes, checkpoint/request metrics code, the `ctxopt` crate, the
CLI/UDS `proxy.*` command family, spawn-path env injection, the per-agent
`proxyEnabled` UI toggle, and the design-host prototype dashboard.

Inventory below was produced by an exhaustive sweep at 8e9318a (clean tree).
Line numbers are anchors, not gospel — re-locate by symbol if drifted.

### A. Delete outright

Rust runtime (`src-tauri/src/engine/runtime/`):
- `ctx_proxy.rs` (7167 lines — core: axum listener, ProxyRuntime, all orchestration)
- `summary.rs` (815 — H1 generation client)
- `quality.rs` (3210 — H2 judge/probe client)
- `quality_audit.rs` (962 — H2 human-audit web UI; binds a SECOND ephemeral loopback listener at :276)
- `quality_fixtures.rs` (266) and the whole `quality_fixtures/` dir (`h2-adversarial-v1.json` include_str asset)
- `count_tokens.rs` (1122 — proxy-exclusive; verified zero importers outside deleted files)

Rust repo (`src-tauri/src/engine/repo/`):
- `proxy_metric.rs` (128), `proxy_checkpoint_metric.rs` (303),
  `proxy_summary_metric.rs` (2090), `proxy_quality_metric.rs` (2335)

Rust commands:
- `src-tauri/src/engine/commands/proxy.rs` (1049 — 13 handler fns)

Workspace crate:
- `src-tauri/crates/ctxopt/` — the whole directory (2325 lines, 10 files).
  Zero consumers outside deleted files. `crates/codeintel` is UNRELATED — stays.

Design-host prototypes (runtime-globbed, cannot break the build):
- `design/screens/proxy-checkpoint.tsx`
- `design/components/CheckpointHeader.tsx`, `VerdictBanner.tsx`,
  `QDistributionChart.tsx`, `QTrendChart.tsx`, `SamplesTable.tsx`
- `design/lib/checkpointMetrics.ts`

### B. Edit surgically

1. `src-tauri/src/engine/runtime/mod.rs` — drop `pub mod` for count_tokens,
   ctx_proxy, quality, quality_audit, quality_fixtures, summary + their
   `#[allow(dead_code)]` attributes and lane comments (:25-:41). CAREFUL: the
   attribute at :36 belongs to quality_fixtures, NOT to sandbox_config (:38).
2. `src-tauri/src/engine/repo/mod.rs` — drop the four `pub mod proxy_*` lines
   + attached comments/attributes (:29-:38).
3. `src-tauri/src/engine/commands/mod.rs` — drop `pub mod proxy;` (:16).
4. `src-tauri/src/lib.rs` — drop the proxy spawn block (:116-119: comment +
   `proxy_state` clone + `spawn(engine::runtime::ctx_proxy::serve(...))`).
   No invoke-handler change (only `ipc`/`system_accent` are registered).
5. `src-tauri/src/engine/state.rs` — drop the `ctx_proxy` field (:62-63) and
   BOTH constructors: `AppState::new()` (:92) AND `#[cfg(test)] for_tests()`
   (:187). Missing the second breaks only the test build — `cargo check`
   won't catch it.
6. `src-tauri/src/engine/router.rs` — remove `proxy` from the braced use list
   (:3) and all 13 `proxy.*` arms + section comment (:154-167).
7. `src-tauri/src/engine/commands/cli.rs` — drop the `"proxy" =>` dispatch
   (:820-821), drop ", proxy" from the allowed-families error string (:825),
   delete `fn map_proxy_argv` entirely (:964-~1231; next fn is `code_usage`),
   delete the proxy test block (:3772-:4090 — 6 tests + 2 helpers
   `summary_on_argv`/`quality_on_argv`). Check `take_switch` (:955) for
   remaining callers before deleting it too.
8. `src-tauri/src/bin/conclave-cli.rs` — drop the proxy USAGE lines
   (:131-139) and the test `proxy_commands_pass_through_for_plain_cli_exec`
   (:5740-5752). There is no proxy enum — argv is forwarded verbatim.
9. `src-tauri/src/engine/commands/instance.rs` — delete `fn proxy_env`
   (+doc, :104-136), `fn append_proxy_env` (:138-145), the spawn-time block
   computing `proxy_env_vars`/`proxy_port` (:761-773), the `proxy_port`
   argument at :847, the second arg of `codex_socket_overrides(sock, proxy_port)`
   (:884), the append block + its ordering comment (:956-960), and the test
   `proxy_env_defaults_on_for_claude_off_for_codex` (:1951-2009).
   Do NOT touch :1188 — "a genuine proxy for the conversation" is unrelated prose.
10. `src-tauri/src/engine/runtime/sandbox_config.rs` — EDIT, never delete:
    remove the `proxy_port: Option<u16>` parameter from all four public fns
    (`codex_socket_overrides`, `claude_sandbox_settings`,
    `claude_agent_settings`, `write_claude_settings`) plus their proxy doc
    lines and the two `if proxy_port.is_some()` loopback-allowlist blocks
    (:56-60, :129-146). The UDS socket hole and rtk PreToolUse hook logic in
    the same functions MUST survive. All call sites are positional — re-arity
    every caller and every test in the file; delete the 4 proxy-domain tests
    (:578-~635).
11. `src-tauri/src/engine/repo/agent_definition.rs` — remove the
    `proxy_enabled` field from all three structs (:122-126, :186-190,
    :264-266), the COLS entry (:220) AND shrink `const COLS: [&str; 23]` to
    22 (:198) — hard compile error otherwise, the hand-written SELECT at :299
    (in-sync warning at :292), both binds (:451-454, :569-572), the struct
    init (:483), the test `proxy_enabled_tristate_roundtrip` (:1103-1160) and
    scattered assertions (:627, :661, :691, :722, :766, :795, :876, :937, :970).
12. `src-tauri/src/engine/commands/agent.rs` — drop `proxy_enabled` from the
    save-request struct (:87-92) and its passthrough (:321).
13. `src-tauri/Cargo.toml` — remove deps `axum`, `rand_chacha`, `rand_core`
    (proxy-exclusive, verified), the `ctxopt` path dep (:51) and the
    `crates/ctxopt` workspace member (:61). KEEP `sha2`, `futures-util`,
    `reqwest` (shared with uds/design_host/memory/task/fusion/provider/cli).
    Commit the resulting `Cargo.lock` change.
14. UI (`src/`):
    - `src/components/Builder.tsx` — drop the `proxyEnabled` state hook
      (:207-208), the save-payload line (:480-481), and the whole
      "Context proxy" Toggle block (:1292-1304).
    - `src/ipc/commands.ts` — drop `proxyEnabled?: boolean;` (+doc, :91-93).
    - `src/ipc/types.ts` — drop `proxyEnabled?: boolean | null;` (+doc, :72-74).
    - `src/fixtures/scenarios/data.ts` — drop both fixture keys (:58, :88);
      leaving them is a TS excess-property error after the type change.

### C. Explicitly OUT of scope — do not touch

- ALL `migrations/*.sql` (0019-0026 created the proxy tables/columns) and
  `src-tauri/src/engine/db.rs` (include_str!s all eight). Deleting a migration
  breaks every existing DB's version chain. The six tables + the
  `agent_definition.proxy_enabled` column become dormant schema — that is the
  intended end state.
- Anything `rtk_*` (`rtk_enabled`, `rtk_hook`, `resolve_rtk_bin`,
  `codex_rtk_hook_override`). "rtk-parity" in comments is analogy, not coupling.
- `src-tauri/src/engine/runtime/transcript_context.rs` (rtk meter, zero proxy refs).
- `docs/superpowers/{plans,specs}/*` — historical record, including this file's
  siblings about H1/H2.
- `src-tauri/tests/lane_guard.rs`, `src-tauri/tests/fastembed_spike.rs` — zero
  proxy references, no changes.
- Test-Proxy-on / Test-Proxy-off agent definitions (lifecycle handled by the
  lead after merge, not in this lane).

## Risk ledger

- The `_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL=1` + `ANTHROPIC_BASE_URL` env
  pair in `instance.rs` is atomic — both go together (item B9 removes both).
- `sandbox_config.rs` is the highest-churn edit: positional arity change across
  ~10 tests + 2 production call sites. Take it slowly, compile often.
- `cargo check` alone will NOT catch `state.rs for_tests()` or any test-only
  breakage — the gates below include `cargo test`.
- Line numbers drift the moment deletes start — anchor by symbol name.
- Cargo.lock will change (crate member + 3 deps removed) — commit it; do not
  regenerate anything else.

## Gates before READY (run all; record via `conclave task gate`)

From `src-tauri/` (cargo exits 101 from repo root — known):
1. `cargo check` — clean.
2. `cargo test` — full suite green (compiles all test code; catches S9-class misses).
From repo root (fresh lane worktree needs `pnpm install` first — known):
3. `pnpm build` — TS typecheck + bundle clean.
4. `pnpm uishot builder` — then OPEN the PNG with the Read tool and LOOK:
   the Builder settings pane must render intact with the "Context proxy"
   toggle gone and no layout hole. Attach the shot path in the READY note.
   (UI Pixel Gate, CLAUDE.md standing protocol. Check :1420 for foreign vite
   servers first — `lsof -nP -iTCP:1420 -sTCP:LISTEN`.)
5. Zero-reference sweep (amended per Dew's challenge 73b53c5a — the original
   raw grep could never pass: `db.rs:243` `include_str!`s the migration
   filename `0020_agent_proxy_enabled.sql`, which section C mandates keeping):
   ```
   bash -c 'hits=$(grep -rn "ctx_proxy\|ctxopt\|proxy_enabled\|proxyEnabled\|18787" src-tauri/src src design --include="*.rs" --include="*.ts" --include="*.tsx" | grep -v "migrations/0020_agent_proxy_enabled.sql"); [ -z "$hits" ] && echo PASS || { echo "$hits"; exit 1; }'
   ```
   — must print PASS and exit 0. The single excluded line is the
   section-C-mandated migration include_str!, not a live proxy reference.

## After merge (integrator)

- Integrator reruns gates 1-3 + 5 on main, then `lane finish`.
- Lead updates bb `plan:hybrid-h1` (removal landed), saves the closure memory,
  reports to the human, and handles Test-Proxy agent retirement separately.
