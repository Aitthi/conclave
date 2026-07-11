# Infinity-Turn Checkpoint — M1 Fix (R8: 3-bucket checkpoint metric)

owner: 4fb2198c-e0d9-4e4b-af9e-d4e72542bace (Detoro, chair) · authority: in-loop
spec: `docs/superpowers/specs/2026-07-11-infinity-turn-checkpoint-design.md` §4/§7.1 + decision-log **R8**
base M1 plan: `docs/superpowers/plans/2026-07-11-infinity-turn-checkpoint-m1.md` (see its top AMENDMENT banner)
challenges: `4f3aa72a` (Detoro) + `c08a3cf1` (Aoki) — both ACCEPTED.

## Problem (measured, not theoretical)
M1 shipped and returned **0 rows** in `proxy_checkpoint_metric`. Root cause, two defects:
- **D1** `checkpoint_gate` maps `CheckpointOutcome::Saturated => None` (`src-tauri/src/engine/runtime/ctx_proxy.rs`, the `match … project(…)` at ~L623) → **no metric row on Saturated**. Spec §4 requires emitting a saturated metric. So "0 rows" cannot be distinguished from "no traffic".
- **D2** the M/L eligibility decision runs in **bytes/4 space**: `est_whole_tokens = ctxopt::estimate::est_tokens(body.len())` (ctx_proxy.rs ~L612) feeds `checkpoint::project()`, which gates `net_saved_tokens > M(40k) && projected_post_tokens <= L(350k)` on that byte estimate. Empirically **bytes/4 overstates real tokens ~4.7×** (18.7–19.0 bytes/real-token measured on real cache-heavy `/v1/messages` bodies: 7.39 MB body vs 394k `cache_read`+504 `input`). Real ~395k-token conversations read as ~1.85M → fail `post <= L` → **always Saturated** → the authoritative `count_tokens` a/b/c sample never runs.

## Fix (authoritative design — R8)
`bytes/4` becomes a **cheap sample-TRIGGER + recorded diagnostic only**, never the M/L authority. Classify **after** `count_tokens`, on real tokens, into **three buckets**, and **always persist a row**.

Trigger (unchanged, cheap): `plan_checkpoint` still returns `None` when `est_tokens(body.len()) <= ceiling` — keep this as the generous byte trigger (over-triggers on purpose). A checkpoint is *sampled* when: byte-est > ceiling **and** ≥1 recoverable candidate outside the tail.

Classification (real tokens, post-count_tokens), `a`=count(original), `b`=count(projected), `c`=count(prefix):
- **`below_ceiling`** — `a <= ceiling` (byte trigger was a false positive). Record `a` + byte diagnostics; no q claim.
- **`eligible`** — `a > ceiling` **and** `S_net = a-b > M` **and** `projected_post = b <= L`. Record full contract.
- **`saturated`** — `a > ceiling` but M or L not met. Record full contract (so the near-miss distribution is visible).

Two thresholds must not be conflated: the **byte-space trigger** (gates count_tokens spend) vs the **real-token ceiling** compared to `a`. Async queue-drop stays the separate `checkpointSamplesDropped` counter, never a metric row.

## Steps (TDD; use superpowers:test-driven-development)

### 1. ctxopt — decouple M/L from projection; add pure classifier
`src-tauri/crates/ctxopt/src/checkpoint.rs`
- Add `pub fn build_projection(messages, plan, est_whole_tokens) -> Option<Projection>`: returns `Some(Projection)` when `plan.candidates` non-empty (the current `project()` body minus the M/L branch), `None` otherwise. NO M/L.
- Add pure classifier:
  ```rust
  pub enum CheckpointClass { BelowCeiling, Eligible, Saturated }
  pub fn classify(a: usize, b: usize, ceiling: usize, m: usize, l: usize) -> CheckpointClass {
      if a <= ceiling { return CheckpointClass::BelowCeiling; }
      if a.saturating_sub(b) > m && b <= l { CheckpointClass::Eligible } else { CheckpointClass::Saturated }
  }
  ```
- Keep `Projection`. **Remove** the old `project()`/`CheckpointOutcome::{Saturated,Eligible}` M/L pre-gate (or leave `project` only if a caller still needs it — the gate must stop using it). Move the old adversarial M/L unit tests onto `classify` with token inputs.
- **Tests:** `classify` all 3 buckets incl. boundaries (`a==ceiling`→BelowCeiling; `a-b==M`→not Eligible; `b==L`→Eligible; `b>L`→Saturated). `build_projection`: non-empty candidates→Some, empty→None, deterministic (same input→same output).

### 2. ctx_proxy — gate builds a job whenever triggered; classify after count_tokens
`src-tauri/src/engine/runtime/ctx_proxy.rs`
- `checkpoint_gate`: when `plan_checkpoint(...)` is `Some` and `build_projection(...)` is `Some`, **always** return a `CheckpointJob` (drop the `Saturated => None` branch). Carry `ceiling`, `m`, `l`, and byte diagnostics on the job so the sampler can classify + record without recomputing.
- `sample_checkpoint`: after `a/b/c`, call `ctxopt::checkpoint::classify(a, b, ceiling, m, l)`; compute `S_net=a-b`, `R=a-c`, `q = R>0 ? S_net/R : 0.0`, `projected_post=b`. Persist a row with `outcome` = the bucket string. Keep the existing **fail-open** count_failure path (record `count_failure=1` + byte diagnostics; set `outcome="count_failure"`).
- (nice-to-have, keep scope tight) est>ceiling but candidates empty → optionally record a cheap `outcome="saturated"` byte-only row (no count_tokens). MUST-have is the candidates-present path.

### 3. Metric schema + writer
- New migration `src-tauri/src/engine/migrations/0022_proxy_checkpoint_outcome.sql`:
  `ALTER TABLE proxy_checkpoint_metric ADD COLUMN outcome TEXT NOT NULL DEFAULT 'eligible';`
  (default keeps existing/legacy rows valid; new rows always set it explicitly.)
- `src-tauri/src/engine/repo/proxy_checkpoint_metric.rs`: add `pub outcome: String` to `CheckpointMetricInsert`; add `outcome` to the INSERT column list + `.bind(m.outcome)`.
- If a read-row struct exists in `src-tauri/src/engine/db.rs`, add the field there too.
- **Test:** migration applies on a fresh DB; insert round-trips `outcome`.

### 4. checkpoint-report surfaces the buckets (this is the deliverable's visibility)
`src-tauri/src/engine/commands/proxy.rs` — `checkpoint-report` groups/counts by `outcome` (below_ceiling / eligible / saturated / count_failure) and, for eligible+saturated, reports q distribution (min/median/max) + projected_post band. This is what turns raw rows into the GO/NO-GO signal.

## Gates (implementer, BEFORE READY)
- `cd src-tauri && cargo test -p ctxopt -p conclave` (run from `src-tauri/`; root fails "could not find Cargo.toml"). Do NOT pipe through `tail` (hides the conclave_lib result) and do NOT append `echo` after a pipe (masks cargo exit). Write full log + `$?` to separate files, then read them.
- `cd src-tauri && cargo clippy --all-targets` — scope failures to THIS boundary; the repo has known pre-existing whole-workspace clippy drift OUTSIDE this boundary (e.g. `crates/ctxopt/src/analyze.rs:127` type_complexity) — do not chase it; note it in the gate.
- Fresh lane worktree: `cargo build` once so deps are present.
- Record: `conclave task gate <ws> infinity-turn-checkpoint-m1-fix -- cd src-tauri && cargo test -p ctxopt -p conclave`.

## Global constraints (inherit)
- **NEVER alter forwarded bytes** — log mode only; the measurement path reads the body by reference and produces only metric rows.
- Async sampling **never blocks/delays** forwarded traffic even under count_tokens rate-limit/latency; keep the existing 60s sample cooldown + permit gate.
- Fail-open: any parse/count error records a row (count_failure), never drops the forwarded request.
- No secrets in rows/logs; credentials lifted from headers in-flight only.

## LIVE re-run (post-merge, Detoro coordinates — NOT an implementer gate)
After merge + a human rebuild/relaunch (proxy ceiling+checkpoint reset on relaunch), set `conclave proxy checkpoint on` + a low `ceiling`, drive real traffic, and confirm rows appear tagged with all buckets; then read `checkpoint-report` for the q distribution → the actual M1 GO/NO-GO verdict.

## Boundary
`src-tauri/crates/ctxopt/src/checkpoint.rs`, `src-tauri/src/engine/runtime/ctx_proxy.rs`, `src-tauri/src/engine/repo/proxy_checkpoint_metric.rs`, `src-tauri/src/engine/migrations/0022_proxy_checkpoint_outcome.sql`, `src-tauri/src/engine/commands/proxy.rs`, `src-tauri/src/engine/db.rs`

## Risk ledger
- `est_tokens` divisor stays 4 — do NOT retune it globally (it's the ceiling-trip proxy + a diagnostic; changing it perturbs unrelated ceiling semantics). The fix is to stop using it for M/L, not to recalibrate it.
- `plan_checkpoint`'s `est <= ceiling => None` stays — it's the byte trigger, intentionally generous.
- Watch `q` div-by-zero when `R=0` (identical prefix) — guard to 0.0.
- The old M/L unit tests in checkpoint.rs will fail once M/L leaves `project` — move them to `classify`, don't delete the coverage.
