# Agent activity indicator — who is working, animated + CLI-visible

**Date:** 2026-07-04 · **Lead:** Detoro (bfb737ff) · **Implementer:** Dew ·
**Reviewer:** Mellow · **Design:** Arta (animation proto + gate) ·
authority: in-loop (human-directed: "ทำ Animation ตรง agent ฝั่งซ้าย
ว่าใครกำลังทำงานอยู่ และให้ agent เรียกดูผ่าน cli Conclave ได้ด้วย
เพิ่มให้ Leader เช็คได้ น่าจะต้อง Update skills ด้วย")

## What "working" means (lead ruling R-act-1)

An agent is **working** iff its live backend emitted PTY/stream output within
the last **5 seconds** (`WORKING_WINDOW`). Rationale: a CLI agent doing real
work (tool calls, spinner, streaming) continuously repaints; one idle at its
prompt emits nothing. This is the only honest signal that needs zero
cooperation from the child process. It applies to `cli` and `chat` backends
alike (both stream through the same forwarder).

- `status: "running"` (session alive) stays untouched — working is a NEW,
  orthogonal, in-memory signal. Nothing is persisted; an app restart resets
  it (correct: no sessions are live then).
- **Risk to verify empirically (see Risk ledger):** an idle Claude Code TUI
  must actually be quiet. Verify before building the UI on top.

## Architecture (lead ruling R-act-2: pull-model, no Rust timers)

The frontend ALREADY receives a `session:output` event per chunk, so the UI
can drive its own working/quiet transitions with a 5 s timeout — no new
event, no Rust-side debounce or sweeper (tokio `time` feature stays off,
per pty.rs's existing constraint). The Rust side only (a) records
last-activity per instance in the in-memory `Runtime`, and (b) reports it
through `instance.list`, which `conclave agent list` already maps to
(commands/cli.rs:85-93) — the CLI gets the feature for free.

## Task B1 — Runtime activity map (`src-tauri/src/engine/runtime/mod.rs`)

- Add to `Runtime`: `activity: Mutex<HashMap<String, std::time::SystemTime>>`
  (same poison-recovery idiom as `sessions`; guard never held across await —
  all ops are quick map ops, matching the module's concurrency contract).
- `pub fn mark_activity(&self, instance_id: &str)` — insert `SystemTime::now()`.
- `pub fn last_activity(&self, instance_id: &str) -> Option<std::time::SystemTime>`.
- Remove the entry in `unregister` and `unregister_epoch` (the paths that
  drop a `LiveHandle`) and in `Drop for Runtime` — a dead session must read
  as not-working immediately.
- Unit tests alongside the existing ones: mark→read roundtrip; unregister
  clears; unknown id → None.

## Task B2 — stamp activity in the forwarder (`src-tauri/src/engine/commands/instance.rs`)

In `forward_session_output`'s recv loop (instance.rs:677), first statement
per chunk: `runtime.mark_activity(&instance_id);`. The forwarder already
holds `runtime` and `instance_id`. One mutex op per ≤4 KB chunk is within
the existing backpressure budget.

## Task B3 — expose through `instance.list` (+ wire types)

- `WorkspaceAgentWithSkills` (repo/workspace_agent.rs:130) gains three
  ADDITIVE optional fields (camelCase on the wire, all
  `skip_serializing_if = "Option::is_none"`), populated by the HANDLER —
  the repo layer stays DB-pure and initializes them to `None`:
  - `working: Option<bool>` — `Some(true/false)` only for live instances.
  - `last_activity_at: Option<String>` — ISO-8601 UTC of the last chunk.
  - `session_id: Option<String>` — the LIVE session id from
    `runtime.session_id(...)`, so the frontend can map `session:output`
    events (which carry only sessionId) back to a roster row.
- `commands::instance::list` (instance.rs:66): after fetching rows, for each
  row where `state.runtime.is_live(&row.id)`: set `session_id`,
  `last_activity_at`, and `working = age(last_activity) <= WORKING_WINDOW`.
  Define `const WORKING_WINDOW: Duration = Duration::from_secs(5);` here
  with a comment that Roster.tsx mirrors the value.
- Frontend `src/ipc/types.ts` `WorkspaceAgent` (types.ts:101) gains
  `working?: boolean; lastActivityAt?: string; sessionId?: string`.
- CLI: NO changes — `conclave agent list <ws>` prints the enriched JSON.

## Task B4 — skills + agent context blurb

- `src-tauri/skills/leadership/SKILL.md`, section "Idle time is oversight
  time": add one bullet — `conclave agent list <ws>` now reports
  `working`/`lastActivityAt` per agent; read it BEFORE interrupting an
  implementer or declaring a lane stalled (a working agent gets left alone;
  a quiet one with an open claim is the thing to chase).
- `src-tauri/src/engine/agentctx.rs` (:65 blurb): extend the `agent list`
  sentence with "each entry reports working=true while that agent is
  actively emitting output". Keep it to one clause — the sidecar is loaded
  every session.
- Note in progress key: existing agents see skill changes only after
  relaunch ("Restart to apply" already handles this).

## Task F1 — Arta: animation proto (parallel with B1-B4)

Proto the Roster agent-row working state in `.arta` (new screen or extend an
existing one). Constraints (lead):

- Three visually distinct states per row: **working** (animated), **running
  but quiet** (current static green dot), **idle** (current gray). The
  animation must read at a glance in a 266px sidebar, no layout shift, and
  must not fight the hover-reveal remove affordance (Roster.tsx:158-164 —
  the dot area hides on hover).
- Must define a `prefers-reduced-motion` fallback (static but still
  distinguishable from quiet-running).
- Design freedom otherwise: pulsing dot, avatar ring, shimmer — Arta's call.

## Task F2 — Dew: Roster animation (AFTER F1 proto lands + lead approves)

`src/components/Roster.tsx`:

- Seed: `RosterEntry` gains `working: boolean` + `sessionId?: string` from
  the (now enriched) `instance.list` rows in the fetch join (:270-291) and
  the `session:status` refetch patch (:315-331).
- Live updates: subscribe `useEvent<SessionOutputEvent>(EVENT_NAMES.
  sessionOutput, ...)`; on an event whose `sessionId` matches an entry, set
  that entry working and re-arm a per-instance 5000 ms timeout
  (`WORKING_WINDOW_MS = 5000`, mirror of the Rust const) that clears it.
  Timeouts live in a `useRef<Map<string, number>>`; clear them all on
  unmount and on workspace switch.
- Render the gated proto's states. Working animation only for entries whose
  session is live; never for `idle`.
- `pnpm vite build` + `npx tsc --noEmit` green; rail/hub untouched.

## Out of scope

- No DB migrations, no new events, no CLI subcommands, no persistence of
  activity, no per-message "typing" semantics in Chat.

## Risk ledger

- **Idle-TUI noise (top risk):** if an idle Claude Code repaints
  periodically, everyone reads as permanently working. Dew verifies FIRST,
  via an ISOLATED harness — spawn a real `claude` process through
  `pty::spawn_cli` in a scratch dir (cargo-level, ignored-by-default test or
  a one-off bin; never the shared app/DB), let the initial paint settle,
  then record chunk timestamps+sizes for ~60 s of idle. (Amended after
  Dew's escalation: the original "watch a live agent via conclave agent
  list" needed the enriched build installed, which conflicts with the
  single-combined-restart directive; deferring the check past that restart
  would push F2 into a second build — worse. Isolated PTY observation
  measures the same phenomenon without either cost.) If noisy: raise the
  window and/or ignore chunks < 16 bytes — escalate to lead with the
  observed cadence before choosing.
- **MISS, found post-ship (bb plan:working-false-positive, human smoke report
  2026-07-04): interaction-driven noise, not idle noise.** This ledger only
  checked whether an idle agent repaints on its OWN — it never checked
  whether the APP ITSELF provokes repaint chunks from an idle agent.
  Terminal.tsx's mount-jiggle (forced resize on every tab mount) and
  wheel-scroll arrow-key injection both write to the live PTY and provoke a
  redraw; `mark_activity` stamped unconditionally on every chunk, so those
  self-inflicted repaints read back as "working". Proven empirically: a
  resize jiggle alone (no stdin) provoked 2 repaint chunks from an idle
  `claude` (`runtime::pty::tests::idle_claude_repaints_on_resize_jiggle`).
  Fix: an engine-side rolling echo-suppression horizon (`ECHO_SUPPRESS_MS =
  500`, `Runtime::mark_activity_gated`) armed by `send_stdin`/`resize` —
  `bus::SessionOutput` gains `activity: bool`; Roster ignores non-activity
  chunks. Lesson for future risk ledgers on this class of feature: idle-noise
  checks must be paired with an interaction-noise check (does the UI's own
  event handling write to the channel being measured?).
- The `session:output` payload delivers full chunks to every listener;
  Roster's handler must do a cheap map lookup only — no state churn when
  the entry is already working (skip the setState if unchanged, re-arm the
  timer only).
- `noUnusedLocals`/dead-code lints on both sides; clippy `-D warnings` is
  the repo's Rust bar (Mellow reruns it).
- Rust gate: `cargo test -p conclave` (lib tests) + `cargo clippy -D
  warnings`; frontend gate as usual.

## Gate chain

1. Dew: B1→B4 (Rust-first; B2's empirical idle check before F2), progress
   at `progress:agent-activity-indicator`.
2. Arta: F1 proto in parallel → lead approves direction → Dew F2.
3. Mellow: review ALL commits vs this plan (Rust + TS together is fine).
4. Arta: design gate on the live Roster vs the F1 proto →
   `bb review:agent-activity-design`.
5. Lead: rerun gates, ONE combined rebuild+install (human directive: single
   restart) → human smokes hub + rail round 3 + activity animation together.

Escalations: design/spec → Detoro (final). Implementation judgment within
plan → Dew, logged in the progress key.
