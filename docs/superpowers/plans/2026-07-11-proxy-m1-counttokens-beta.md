# Ctx-proxy M1 — count_tokens must forward `anthropic-beta` (fix the blind instrument)

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro) · authority: in-loop
impl: Tiësto (00b076a0) · co-design cc: Aoki (1b074885)
base: main @ 751e355

## Why (diagnosis — reproduced live)

After the 2026-07-11 rebuild+relaunch, the M1 live re-run produced **5/5 `count_failure`**
in `proxy_checkpoint_metric` (rows 1–5, all `model=claude-opus-4-8`, one conversation
sampled every ~80 s — the lead's own agent, whose context is well under 200k tokens).
Every checkpoint sample returns `outcome=count_failure`, `q=0` → the instrument yields
**zero q data** and is blind in exactly the 200k–500k degradation zone the proxy targets.

Root cause (code-confirmed):
- `credential_from_headers` (`ctx_proxy.rs:649`) lifts only `x-api-key`,
  `authorization`, `anthropic-version`.
- `apply_cred` (`count_tokens.rs:74`) applies only those.
- **`anthropic-beta` is never forwarded.** The fleet agents authenticate with OAuth
  Bearer tokens (`ANTHROPIC_BASE_URL=http://127.0.0.1:18787` → upstream
  `https://api.anthropic.com`); Claude Code's OAuth requires an `anthropic-beta` header
  to be accepted, and the `[1m]` models additionally require a context beta. Stripping it
  makes `POST /v1/messages/count_tokens` fail for every sample.
- The exact HTTP status is currently invisible: the GUI app's stderr → `/dev/null`
  (`eprintln!` at `ctx_proxy.rs:758`), so we cannot see the failure body.
- `preflight()` (`count_tokens.rs:122`) is `dead_code` — the plan itself flagged
  "no live credential check wired in M1 — escalation to the lead." This is that gap.

## Ruling (Detoro, in-loop) — reconciles security containment ea3df57c

The M1 credential allowlist (ea3df57c) deliberately lifted only auth headers to prevent a
changed global upstream from retargeting the credential. **Amendment:** `anthropic-beta`
is added to the lifted set. Rationale: it is **not a secret**, it is **required for the
lifted auth to be accepted** by the *same captured upstream* (`job.upstream`, never
re-read from global state), and forwarding it does not affect upstream-retargeting
containment. Spec §7.1 must be amended to list `anthropic-beta` in the lifted set with
this rationale.

## Changes (exact)

### 1. Forward `anthropic-beta` — `count_tokens.rs`
- Add `anthropic_beta: Option<String>` to `CountCredential` (comment: `// anthropic-beta`).
- In `apply_cred`, when present, add header `anthropic-beta` with the verbatim value.
  Forward the **whole** header value (it may be a comma-separated list; do not split).

### 2. Lift it at the gate — `ctx_proxy.rs`
- In `credential_from_headers`, populate `anthropic_beta` from the `anthropic-beta`
  request header (same `get(..)` pattern as the others).

### 3. Make the failure diagnosable (so we never guess again)
- `count_tokens.rs` non-success arm: **NEVER persist the raw response body.** The repo
  invariant at `proxy_checkpoint_metric.rs:3-5` forbids request/response bodies entering
  this repo. Instead parse the known Anthropic error shape
  (`{"type":"error","error":{"type":…}}`) and build a **content-free** Err of
  `http-status + error.type` only, e.g. `format!("count_tokens HTTP {status}: {error_type}")`
  → `"count_tokens HTTP 400: authentication_error"`. If the body does not parse as that
  shape, record **status-only** (`"HTTP {status} (unparsed)"`) — never raw bytes.
  `error.type` is a bounded enum (authentication_error / invalid_request_error /
  not_found_error / …), so it distinguishes auth-scope vs beta vs model-id vs too-long
  while carrying no user content. Optional: append the opaque `request-id` response
  header if trivially available. (Amended per security challenge 76dbd0ed — Aoki found,
  Tiësto evidenced the repo invariant; original plan wrongly authorized a raw ≤200-char
  body. The spec §7.1 wording and the migration-file comment must match: "status +
  error.type", NOT "≤200-char body".)
- Persist it durably (stderr is `/dev/null`): add nullable column
  `error_snippet TEXT` to `proxy_checkpoint_metric` via a NEW migration file
  `src-tauri/src/engine/migrations/0023_proxy_checkpoint_error_snippet.sql`
  (`ALTER TABLE proxy_checkpoint_metric ADD COLUMN error_snippet TEXT;`).
  **The file alone does nothing** — migrations are applied by version-gated
  `include_str!` blocks in `src-tauri/src/engine/db.rs`. You MUST add a
  `if version < 23 { include_str!("migrations/0023_proxy_checkpoint_error_snippet.sql"); PRAGMA user_version = 23; }`
  block (mirror the existing 0022 block) **and bump EVERY `assert_eq!(version, 22 …)`
  in db.rs's migration tests to 23** — there are FIVE sites (main: db.rs:425, 676,
  879, 970, 1080); do NOT rely on a named subset, `grep -n "assert_eq!(version, 22"
  src/engine/db.rs` and bump every hit. Leave the migration BLOCK lines (`if version
  < 22`, `PRAGMA user_version = 22`) — you ADD a parallel `< 23` block, you don't edit
  those. Miss an assertion → the column still lands but those tests go red; miss the
  `< 23` block entirely → the column is never created and `insert()` fails at runtime. In
  `sample_checkpoint`'s `Err(error)` arm, set `row.error_snippet = Some(error)`;
  `None` on success. Wire the column through `CheckpointMetricInsert` + `insert()`
  in `repo/proxy_checkpoint_metric.rs`. Do NOT put it in `checkpoint-report`
  output (keep that schema stable); it is for SQL diagnosis only.

### 4. Regression test — `count_tokens.rs` tests
- Extend the fake-upstream test so the fake asserts the incoming request carries the
  `anthropic-beta` header when the `CountCredential` sets it, and 400s if absent.
  Assert `count_tokens` succeeds with beta present.
- On the 400 path, the fake returns an error body containing a distinctive marker in
  `error.message` (e.g. a fake "secret-xyz"); assert the surfaced Err **contains
  `error.type`** and **does NOT contain** that message marker — proving the raw body
  never reaches `error_snippet` (guards challenge 76dbd0ed).

## Boundary
(Amended 2026-07-11 per Tiësto challenge cf0d3c06 — ACCEPTED. Original snapshot had
the wrong migration dir and omitted db.rs; corrected here. Aoki independently concurred.)
- `src-tauri/src/engine/runtime/count_tokens.rs`
- `src-tauri/src/engine/runtime/ctx_proxy.rs`
- `src-tauri/src/engine/repo/proxy_checkpoint_metric.rs`
- `src-tauri/src/engine/migrations/0023_proxy_checkpoint_error_snippet.sql` (new)
- `src-tauri/src/engine/db.rs` (register 0023: `version < 23` include_str! block +
  `PRAGMA user_version = 23`; bump the 22→23 `user_version` test assertions)
- `docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md` (§7.1 amend)

## Gate (run from `src-tauri/`)
`cargo test -p ctxopt -p conclave` — full log + `$?` to files, READ them (do NOT pipe
through tail; the `conclave` lib test line hides). Record via
`conclave task gate <ws> proxy-m1-counttokens-beta -- cargo test -p ctxopt -p conclave`.
No UI touched → no uishot gate.

## Acceptance (lead re-measures after human rebuild+relaunch)
After merge + rebuild + relaunch, re-arm (`proxy checkpoint on; proxy ceiling 100000`),
drive traffic, then `proxy checkpoint-report`: `count_failure` must drop to ~0 and
samples classify into `below_ceiling|eligible|saturated`. If any `count_failure` remains,
`SELECT error_snippet FROM proxy_checkpoint_metric WHERE count_failure=1` now shows the
real HTTP status/body to pivot on.

## Risk ledger
- If, after forwarding beta, failures persist → `error_snippet` reveals the true cause
  (auth scope, model id, upstream). The snippet step is the safety net; do not skip it.
- The `[1m]` model string is stripped by Claude Code before the API (DB rows show
  `claude-opus-4-8`, not `[1m]`) — model id is NOT the suspected cause; do not "fix" it.
