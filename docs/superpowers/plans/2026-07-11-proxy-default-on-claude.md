# Plan: context proxy default-ON for Claude agents (rtk-parity)

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop
implementer: Dew · reviewer: Mellow · escalation: Detoro

## Goal (human directive, 2026-07-11)

"แก้ให้ claude agent ทุกตัวติด proxy ตั้งแต่ spawn … ให้ตั้งได้แบบ rtk แต่ default
เปิด" — every **Claude** agent routes through the loopback context proxy from
spawn WITHOUT per-agent opt-in, exactly mirroring how `rtk_enabled` already
works (settable per-agent, but NULL/absent defaults ON). This unblocks the
proxy Phase-1 measurement, which has stalled because only 2 hand-opted agents
ever routed through the proxy.

## Why this is the right shape

- The proxy default is decided at SPAWN in `proxy_env()`, not at agent-save,
  identical to rtk's `rtk_enabled.unwrap_or(true)`. Flipping the spawn default
  retroactively opts in EVERY existing Claude agent whose `proxy_enabled` is
  NULL — no data migration, no per-agent edit.
- `ctx_proxy` is Anthropic-only (`DEFAULT_UPSTREAM = https://api.anthropic.com`,
  rewrites only `/v1/messages`). Routing a codex agent through it (which speaks
  the OpenAI protocol) would BREAK it. So default-ON is gated to Claude; codex
  behavior is left EXACTLY as today (default OFF, explicit opt-in preserved).

## Global constraints (inherit for every step)

- UI copy is English. Terminal/agent messages English; human replies Thai.
- This lane touches `src/` → the **UI Pixel Gate** applies (CLAUDE.md): run
  `pnpm uishot builder`, OPEN and LOOK at the PNG, attach the path in the READY
  note, and record `conclave task gate <ws> proxy-default-on-claude -- pnpm
  uishot builder`. A green exit code alone does NOT count.
- Fresh lane worktree has no node_modules — `pnpm install` once before any
  pnpm/tauri gate.
- Before trusting a uishot, `lsof -nP -iTCP:1420 -sTCP:LISTEN` and kill any
  foreign vite server from another checkout.

## Boundary (only these files)

- `src-tauri/src/engine/commands/instance.rs`
- `src/components/Builder.tsx`
- `src/ipc/types.ts`
- `src/ipc/commands.ts`
- `src/fixtures/scenarios/data.ts`

## Step 1 — Backend: default-ON for Claude (`instance.rs`)

Current (`instance.rs:109-118`):
```rust
fn proxy_env(proxy_enabled: Option<bool>, active_port: Option<u16>) -> Option<(String, String)> {
    if !proxy_enabled.unwrap_or(false) {
        return None;
    }
    let port = active_port?;
    Some(("ANTHROPIC_BASE_URL".to_string(), format!("http://127.0.0.1:{port}")))
}
```

Change to (add a `default_on` param; keep everything else):
```rust
fn proxy_env(proxy_enabled: Option<bool>, default_on: bool, active_port: Option<u16>) -> Option<(String, String)> {
    if !proxy_enabled.unwrap_or(default_on) {
        return None;
    }
    let port = active_port?;
    Some(("ANTHROPIC_BASE_URL".to_string(), format!("http://127.0.0.1:{port}")))
}
```

Update the call site (`instance.rs:669`) — `base` is already `"claude"` or
`"codex"` in scope here:
```rust
let proxy_env_var = proxy_env(def.proxy_enabled, base == "claude", state.ctx_proxy.active_port());
```

Update the doc comment (`instance.rs:104-108`) to state the NEW semantics:
Claude defaults ON (NULL/absent = ON, rtk-parity, overridable to OFF per
agent); codex defaults OFF (unchanged, Anthropic-only proxy). Fail-open on a
down listener is unchanged.

### Step 1 tests (`instance.rs:1714-1726`)

The existing `proxy_env_requires_opt_in_and_active_listener` test encodes the
OLD opt-in default and must be updated for the new signature. Cover exactly:
- `proxy_env(None, true, Some(18787))` → Some (Claude default ON)
- `proxy_env(None, false, Some(18787))` → None (codex default OFF)
- `proxy_env(Some(false), true, Some(18787))` → None (explicit opt-OUT wins for Claude)
- `proxy_env(Some(true), false, Some(18787))` → Some (explicit opt-IN wins for codex)
- `proxy_env(Some(true), true, None)` → None (fail-open, no listener)
Rename the test to reflect default-ON-for-claude semantics.

Gate: `cargo test -p <crate> proxy_env` green (find crate via
`src-tauri/Cargo.toml`).

## Step 2 — Frontend: proxy toggle, rtk-parity (`types.ts`, `commands.ts`, `Builder.tsx`, `data.ts`)

The backend save path already accepts `proxy_enabled` (`agent.rs:89,318`) but
the TS wire never sends it. Add it, mirroring `rtkEnabled` everywhere:

- `src/ipc/types.ts` — beside `rtkEnabled?` (line ~71), add
  `proxyEnabled?: boolean | null;` with a comment "Claude agents only; NULL =
  ON (rtk-parity). DB `proxy_enabled`."
- `src/ipc/commands.ts` — beside `rtkEnabled?` (line ~90) in the SaveAgentDef
  command payload, add `proxyEnabled?: boolean;`.
- `src/components/Builder.tsx`:
  - state: `const [proxyEnabled, setProxyEnabled] = useState<boolean>(initialDef?.proxyEnabled ?? true);` (default true — mirror rtk line 206)
  - save payload (line ~477): `proxyEnabled: showCliConfig ? proxyEnabled : undefined,`
  - render a toggle mirroring the rtk block (lines 1275-1285), but show it
    **only for Claude** agents (the same `showCliConfig` guard AND the selected
    cli_kind is claude-code — reuse whatever cli-kind variable the rtk/claude
    branch already uses in this component; do NOT invent a new one). Label
    "Context proxy"; helper text: "Routes this agent's Anthropic requests
    through the local context proxy to measure and compress oversized context."
- `src/fixtures/scenarios/data.ts` — add `proxyEnabled: true` (and one `false`)
  to the agent fixtures beside the existing `rtkEnabled` fields (lines 57, 86)
  so the Builder fixture renders the new toggle.

## Step 3 — UI Pixel Gate

- `pnpm uishot builder` → Read `.shots/builder-default.png`, confirm the
  "Context proxy" toggle renders ON for a Claude agent and is absent for a
  codex agent. Attach the PNG path in the READY note.
- `conclave task gate <ws> proxy-default-on-claude -- pnpm uishot builder`.

## Gates before READY (record each via `conclave task gate`)

1. `cargo test` for the touched crate (proxy_env tests + no regressions).
2. `pnpm tsc` / typecheck green (no new TS errors).
3. `pnpm uishot builder` + PNG visually confirmed.

## Risk ledger

- The `base == "claude"` string is load-bearing — it is the ONLY thing keeping
  codex off the Anthropic proxy. If the cli-kind mapping (`instance.rs:563-565`,
  "claude-code" → "claude") ever changes, this gate silently breaks. Assert it
  in the Rust test by feeding `default_on=false` for the codex case.
- Do NOT touch `agent.rs` / `agent_definition.rs` — the DB column and save wire
  already exist; only the TS wire and the spawn default change.
- No DB migration: existing NULL `proxy_enabled` rows become default-ON for
  Claude purely via the spawn-time flip.
