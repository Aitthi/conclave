# CLI: agents can read their own inter-agent message history

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:test-driven-development` — every task is red-test-first, then implement. Steps use checkbox (`- [ ]`) syntax.

owner: bfb737ff-486d-4581-b407-95711d5e07ab (Detoro) · authority: in-loop
escalation: design/spec conflicts → Detoro via `conclave task challenge`; implementation judgment within this plan → implementer, logged as task notes.

## Goal (the human's words)

"Agent ไม่มี Tools ให้อ่าน Message ที่ Agent คุยกัน" — after a context clear/compact an agent
cannot re-read the `tell` messages it exchanged, even though every message is already
persisted. Give the CLI a read path so an agent (or a lead) can pull that history back.

## Why this is small

The backend is DONE. `message.inject` (`tell`) persists every message to the
`inter_agent_message` table. Two read methods already exist and are wired in the router:

- `message.list` `{ instanceId, limit }` → one instance's inbox+outbox, newest-first
  (`engine/commands/message.rs:215`, repo `list_for_instance`).
- `message.listForWorkspace` `{ workspaceId, limit }` → whole workspace, newest-first
  (`engine/commands/message.rs:242`, repo `list_for_workspace`). This is the UI's Chat Hub feed.

The ONLY gap is the CLI: `conclave-cli` exposes `tell`/`send` (write) but no read verb.
This lane adds the read verb, a readable transcript renderer, and opt-in name enrichment.

## Design decisions (settled by Detoro — do not re-litigate; challenge on the task if wrong)

1. **Two subcommands, not one overloaded verb.**
   - `conclave msg list [--limit N]` → the CALLER's own inbox+outbox. Self-keyed on
     `CONCLAVE_INSTANCE_ID`, exactly like `snapshot last` — requires a spawned-agent context.
   - `conclave msg all <workspaceId> [--limit N]` → the whole workspace's traffic. Takes an
     explicit workspace id, needs NO self context, so a lead/researcher can read it from a
     plain terminal too.
   - *Rejected:* a single `msg list [--workspace <ws>]` that flips between self and workspace.
     Mixing self-injection with an override flag complicates `expand_self_args` and muddies the
     mental model. Two narrow subs each map 1:1 to an existing method.

2. **Name enrichment is opt-in via a `withNames` param (default false).**
   Rows carry only instance UUIDs (`fromInstanceId`/`toInstanceId`). A transcript of raw UUIDs
   is nearly useless for attribution. The list handlers gain an optional `withNames: bool`
   (default false). When true they attach `fromName`/`toName` (resolved instance-id → agent
   definition name) to each emitted object. The CLI always passes `withNames: true`.
   - **CRITICAL:** when `withNames` is absent/false the handler output is BYTE-FOR-BYTE what it
     is today — this keeps the UI's `message.listForWorkspace` path (which never sends
     `withNames`) unchanged. Guard this with a test.
   - *Rejected:* (a) unconditionally adding `fromName`/`toName` to the shared
     `InterAgentMessageRow` struct — touches the UI's typed feed and the TS `InterAgentMessage`
     contract for no UI benefit; (b) resolving names client-side — the CLI has no DB access and
     would need extra round-trips.
   - Enrichment lives in the HANDLER (`message.rs`), NOT the repo — the repo `Row` struct and
     its SQL stay untouched, so the file boundary excludes `repo/inter_agent_message.rs`.

3. **Render a chronological transcript, not JSON.** Both `msg` subs use a new
   `OutMode::MsgList`. Rows arrive newest-first (DESC); reverse them to chronological so it reads
   like a conversation. One token-cheap line per message:
   `HH:MM  <FromName> → <ToName>  <text>`, with a `[queued]` marker when `status == "queued"`.
   Fall back to a short id prefix (first 8 chars) when a name is absent.
   - *Rejected:* `OutMode::Json` passthrough — a wall of UUIDs, defeats the purpose.

## Global constraints (every task inherits)

- **TDD:** write the failing test FIRST, watch it fail for the right reason, then implement.
- **UI path is sacred:** any change to `message.rs` list handlers MUST leave the
  `withNames`-absent output identical to today. A test must assert this.
- **Mirror existing idioms:** `expand_self_args` for self-keying (see `tell`/`snapshot last`
  at `conclave-cli.rs:154,178`), `map_argv` match arms for wire mapping (see `send`/`tell` at
  `cli.rs:100,119`), `OutMode` + the render match for output (`conclave-cli.rs:1920,2253`).
- **Human-facing CLI text:** English only (matches every other verb's usage strings).
- No new dependencies. No changes to `src/` (TS/UI) or `repo/inter_agent_message.rs`.

## File boundary (this lane owns exactly these)

- `src-tauri/src/bin/conclave-cli.rs` — `expand_self_args` arm + `OutMode::MsgList` + renderer + tests
- `src-tauri/src/engine/commands/cli.rs` — `msg` match arm in `map_argv` + tests
- `src-tauri/src/engine/commands/message.rs` — `withNames` on both list handlers + enrichment helper + tests
- `src-tauri/skills/tool-map/SKILL.md` — document the two new verbs

## Risk ledger

- **Fragile:** `expand_self_args` runs CLIENT-side (`conclave-cli.rs`); `map_argv` runs
  SERVER-side (`cli.rs`). `msg list` is expanded to `["msg","list",<selfId>, ...rest]` by the
  client BEFORE the server sees it — the server's `map_argv` reads `argv[2]` as the instance id,
  same as `snapshot last` reads `argv[2]`. Get the index bookkeeping right or arg parsing shifts.
- **Fragile:** `--limit` is optional and may appear after the positional. Parse it out of the
  tail rather than assuming a fixed position.
- **Fragile:** name resolution path is `inter_agent_message` id (an instance id) →
  `workspace_agent::get(id)` → `agent_definition::get(inst.agent_def_id).name`. This is the
  exact chain the `inject` handler already uses (`message.rs:157`). Resolve DISTINCT ids once
  into a `HashMap<String,String>`, don't do it per-row (up to 200 rows).
- **Empty result:** an agent with no messages must print a clean "no messages" line, not a
  panic on an empty array or a bare `[]`.

## Tasks

### Task 1 — `withNames` enrichment on the list handlers (server)
- [ ] RED: in `message.rs` tests, assert `list` with `withNames: true` returns objects carrying
      `fromName`/`toName` resolved to the agent definition names; assert `list` WITHOUT
      `withNames` returns the exact same shape as today (no `fromName`/`toName` keys). Same pair
      for `list_for_workspace`.
- [ ] GREEN: add `with_names: Option<bool>` to `ListReq` and `ListForWorkspaceReq`
      (`#[serde(default)]`, camelCase `withNames`). After fetching rows, if `with_names` is
      `Some(true)`, build a distinct-id→name map (reuse the `workspace_agent::get` →
      `agent_definition::get` chain from `inject`) and emit enriched `serde_json::Value` objects;
      otherwise serialize rows exactly as today.
- [ ] Expected: `cd src-tauri && cargo test message` green.

### Task 2 — `msg` wire mapping (server `map_argv`)
- [ ] RED: in `cli.rs` tests, assert `map_argv(["msg","list",<selfId>])` → method
      `"message.list"` with `{ instanceId: <selfId>, withNames: true }` (+ `limit` when
      `--limit N` present); assert `map_argv(["msg","all",<ws>])` → `"message.listForWorkspace"`
      with `{ workspaceId: <ws>, withNames: true }`; assert missing-arg forms return
      `AppError::Invalid`.
- [ ] GREEN: add a `"msg"` arm to `map_argv` handling `list` and `all` subs, parsing an optional
      `--limit`. Mirror the `send`/`tell` arms.
- [ ] Expected: `cd src-tauri && cargo test cli` green.

### Task 3 — self-expansion + render (client `conclave-cli.rs`)
- [ ] RED: in `conclave-cli.rs` tests, assert `expand_self_args(["msg","list"], Some("self1"))`
      → `["msg","list","self1"]`; assert `--limit` is preserved after the injected id; assert
      `expand_self_args(["msg","list"], None)` is `Err` (self required); assert
      `expand_self_args(["msg","all","ws1"], None)` passes through unchanged.
- [ ] GREEN: add a `Some("msg")` arm to `expand_self_args` — `list` requires self and injects it
      at `argv[2]` (before any `--limit`); `all` passes through. Add `OutMode::MsgList`, select it
      for `argv.first() == "msg"`, and implement the renderer: reverse to chronological, print
      `HH:MM  From → To  text` (+ `[queued]`), name-or-short-id fallback, and a "no messages"
      line for an empty array.
- [ ] Expected: `cd src-tauri && cargo test` green (whole crate builds + all tests).

### Task 4 — document the verbs (skill)
- [ ] Add two rows to the Tool Map table in `src-tauri/skills/tool-map/SKILL.md` under a
      Messages/Peers grouping:
      - `conclave msg list [--limit N]` — read YOUR own inter-agent inbox+outbox, newest-first
      - `conclave msg all <workspaceId> [--limit N]` — read the whole workspace's inter-agent traffic
- [ ] No test; this is documentation of the shipped surface.

## Acceptance gate (run before READY)

```
cd src-tauri && cargo test        # all tests green, whole crate compiles
```

Record it: `conclave task gate 11ecf99b-53f4-4c24-b538-b19e5933a9e3 cli-read-messages -- sh -c "cd src-tauri && cargo test"`

Then a manual smoke inside a spawned agent context (optional, note the output in the READY note):
`conclave msg list --limit 5` should print a readable transcript of recent messages with names.

## Reading order for the implementer

1. This plan (decisions → constraints → risk ledger → tasks).
2. The existing idioms named in the risk ledger — read them before editing:
   `conclave-cli.rs:143-304` (`expand_self_args`), `cli.rs:100-138` (`send`/`tell` arms),
   `message.rs:194-256` (the two list handlers), `conclave-cli.rs:1920-2360` (`OutMode` + render).
