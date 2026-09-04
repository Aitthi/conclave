# Inject long messages via bracketed paste (fix: "message arrives head-truncated / only the last chunk")

owner: 30fa04f4-e047-4241-a9ed-f452529952be (Detoro, Lead) · authority: in-loop
requested by: the human, 2026-09-04 ("Fix bug ให้หน่อย … msg มันขาดหายไปใน input ของ terminal")
solo lane — why: diagnosis and fix are inseparable (the fix is ~30 lines once the
root cause is known, and the root cause lives in the lead's context after a
two-hour investigation); the handoff would cost more than the work. Peer review
still required before merge (Mellow, Reviewer).

## Symptom

A `conclave tell` (engine `message.inject`) whose tagged body is longer than
1022 bytes reaches a Claude Code agent with its head missing: the receiver
submits ONLY the last PTY chunk. Screenshot evidence 2026-09-04 13:34: Dew's
terminal shows the submitted user line `❯ .` for a 965-char message from
Detoro. The comms-protocol skill already warns "long ones can arrive with their
head cut off" — this is that bug.

## Root cause (confirmed by reproduction, not by reasoning)

1. `Runtime::send_stdin` writes the whole tagged body to the PTY master in ONE
   `write_all`. macOS XNU `ptcwrite` admits at most `TTYHOG - 2 = 1022` bytes
   into the slave's raw input queue, then blocks the master until the slave
   reads. So a 1023-byte body reaches the CLI as two reads: 1022 bytes, then
   1 byte. A 2206-byte body arrives as 1022 + 1022 + 162.
2. Claude Code's NON-bracketed burst handling keeps only the last read of the
   burst. Observed received size == body mod 1022 in every failing case.

Evidence table (conclave.db `inter_agent_message` vs the receiver's own
`~/.claude/projects/*/*.jsonl` user entries), 2026-09-03/04:

| tagged body bytes | received bytes | expected last chunk |
|---|---|---|
| 1023 (Detoro→Dew 06:33Z) | 1 (`.`) | 1 |
| 1025 (Mellow→Detoro 03:00Z) | reply says "head-truncated" | 3 |
| 1114 | 92 | 92 |
| 1350 | 328 | 328 |
| 2150 | 106 | 106 |
| 2206 | 162 | 162 |
| 2376 | 332 | 332 |
| 2672 | 628 | 628 |

Some long messages DO arrive intact (receiver busy → chunks spaced out), which
is why the human sees it as intermittent ("บ้าง").

Reproduction harness: `scripts/pty-inject-repro.py` (spawns the real CLI in a
PTY, writes the exact `inject` byte pattern — one body write, then bare `\r`
at +40/+120/+300 ms — and reads back what the CLI's own transcript recorded).
Matrix run 2026-09-04 13:49–13:51 local:

| CLI | mode | body bytes | received |
|---|---|---|---|
| claude 2.1.260 | plain | 300 | INTACT |
| claude 2.1.260 | plain | 1023 | `.` (1 byte) |
| claude 2.1.260 | plain | 1500 | 478 bytes, head missing |
| claude 2.1.260 | plain | 2206 | 162 bytes, head missing |
| claude 2.1.259 | plain | 1500 | 478 bytes, head missing |
| claude 2.1.258 | plain | 1500 | 478 bytes, head missing |
| claude 2.1.260 | bracketed | 1023 / 1500 / 2206 / 4008 | INTACT, all four |
| codex 0.153.2 | plain | 1500 | INTACT (codex merges bursts itself) |
| codex 0.153.2 | bracketed | 1500 | INTACT |

Not a 2.1.260 regression: 2.1.258 and 2.1.259 fail identically.

## Decision D1 — text-shaped stdin goes through bracketed paste

Wrap every TEXT write to a PTY backend in the bracketed-paste envelope
`ESC[200~ … ESC[201~`. This is the protocol real terminals already use for a
human paste (xterm.js in `Terminal.tsx` emits it, Claude Code enables it with
`ESC[?2004h` on startup), so the receiver accumulates the whole paste across
PTY reads instead of guessing burst boundaries. Keystrokes (the submit `\r`,
xterm `onData`) stay raw.

Rejected:
- Split the body into <1022-byte writes with sleeps between them — still
  depends on the receiver's burst heuristics and on timing; slower; unverifiable.
- Put the wrap inside `Runtime::send_stdin` for everyone — would wrap the
  Terminal pane's raw keystrokes and escape sequences, breaking arrows/Enter.
- Wrap in the CLI (`conclave tell`) — leaves `snapshot.rs::submit_line` and the
  StdinBar with the same bug.

## Tasks

1. `src-tauri/src/engine/runtime/mod.rs` — `LiveHandle` gains
   `bracketed_paste: bool`; `Runtime::send_stdin_paste(instance_id, text)`
   wraps when the flag is set, otherwise forwards raw (chat/placeholder).
   Test seam: `LiveHandle::for_test_pty` (flag on) beside `for_test` (flag off).
2. `src-tauri/src/engine/runtime/pty.rs` — `spawn_cli` sets the flag.
   `chat.rs` / placeholders leave it false.
3. `src-tauri/src/engine/commands/message.rs` — `inject` sends the tagged body
   via `send_stdin_paste`; the three spaced `\r` stay raw. `send` gains an
   optional `paste: bool` (default false) so the frontend can opt text in.
4. `src-tauri/src/engine/commands/snapshot.rs::submit_line` — same switch.
5. `src/ipc/commands.ts` + `src/components/StdinBar.tsx` — the composer's text
   write passes `paste: true`; the `\r` write does not.
6. `scripts/pty-inject-repro.py` — the harness, committed as the regression
   check that unit tests cannot provide (the receiver is a third-party binary).

Tests (red first): `inject_live_target_retries_submit_cr` asserts the body
write is `ESC[200~[from …] hi ESC[201~` and the CRs are bare; a chat-backend
test asserts no envelope; `send` with `paste: true` on a PTY handle wraps.

## Gates

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
  (in `src-tauri`).
- `pnpm typecheck` (or `pnpm build` if no typecheck script), `pnpm uishot home`.
- Harness: `python3 scripts/pty-inject-repro.py --bin <claude> --cwd <scratch>
  --size 2206 --mode bracketed` → INTACT (already green pre-implementation; the
  engine change reproduces exactly this byte pattern).
- After rebuild+relaunch (human action): one real `conclave tell` > 1022 bytes
  to an idle peer, then read that peer's transcript user entry — full body.

## Risk ledger

- The RUNNING engine keeps the old behaviour until the human rebuilds and
  relaunches Conclave; agents already spawned keep truncating until then.
- Claude Code collapses large pastes to `[Pasted text #N +M lines]` in the
  composer; the submitted transcript entry carries the full text (verified).
- Codex: bracketed paste is the crossterm standard; verified by the harness row
  above — if that row is not INTACT, gate the wrap on `cli_kind == claude-code`.
- Side effect during investigation: the codex harness accidentally accepted
  codex's self-update prompt (0.148.0 → 0.153.2). Old package still on disk
  under `~/.codex/packages/standalone/releases/`; reported to the human.
