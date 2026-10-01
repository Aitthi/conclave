---
status: accepted
---

# Self-triggered restart resets context in place (handoff → /clear → resume)

Supersedes, for the self-triggered path only, ADR 0006's "handoff saved → process killed →
fresh terminal spawned → resume prompt injected" bullet. Everything else in ADR 0006 stands:
the agent is still the trigger, `conclave restart` still prints its instruction instead of
injecting a turn, and the save still gates the tail.

- **`conclave restart` no longer kills the process.** Once the handoff save lands, the engine
  types the harness's own clear command into the agent's live terminal, then the resume prompt.
  The harness's `/clear` already yields a fresh context while the system-prompt layer (identity
  preamble, standing-instructions file pointer) survives it — killing the process only added a
  full CLI boot, dropped shell/browser/MCP state, and flipped the UI idle → running for nothing.
  The browser first-commit gate is re-armed (`browser::mark_resumed`) exactly as on respawn.
- **Scope is the self-triggered path only.** The arm carries a mode (`RestartMode::{Respawn,
  ClearInPlace}`): the human Restart · resume button and the not-live path keep kill → respawn
  → resume byte-for-byte — the button promises a relaunch and a human may use it on a hung
  process.
- **Clear command per `cli_kind`:** only `claude-code` → `/clear`. Every other kind (codex,
  antigravity, custom, planned opencode/muse) arms the respawn tail; the mode is decided at
  arm time, so the instruction `conclave restart` prints describes the tail that will fire.
  The clear tail keeps its own unknown-kind → respawn fallback as defence.
- **Codex `/new` is deferred.** On codex-cli 0.159.3, `/new` in a git checkout (every Conclave
  workspace is one) opens a "Where should the new conversation run?" picker; the resume
  prompt's paste + Enter would be eaten by it (Enter selects "Current checkout", the pasted
  text is lost) and the agent would land in a fresh chat with no resume prompt (challenge
  32c4ae63). Follow-up: verify live whether an extra Enter after `/new` yields a usable fresh
  chat and whether the next paste survives, then flip `clear_command_for("codex")`.
- **Verified live** on Claude Code 2.1.286: a bracketed-paste `/clear` + Enter runs as the
  local command (a new session starts) both in an idle TUI and when queued behind a running
  turn; the resume line typed after it arrives as its own first message in the new session.
- **Timing is turn-gated, not guessed.** The save fires from inside the agent's own tool call,
  so the agent is still mid-turn. The tail waits a 2 s floor, then for the PTY to be quiet for
  1.5 s (`Runtime::last_activity`, which ignores the echo of our own input; capped at 60 s),
  types the clear command, waits a 3 s floor plus quiet again (capped at 20 s), then types the
  resume prompt. Both go through the bracketed-paste submit seam every injected prompt uses.
  It re-checks liveness and eligibility first, and never holds the agent lifecycle mutex across
  the waits.

## Rejected alternatives

- **Keep killing the process.** Costs a CLI boot and the agent's shell/browser/MCP state for
  no gain over `/clear`.
- **Respawn with the harness's own session resume (`claude --resume`).** Rejected again for
  ADR 0006's reasons: Conclave doesn't track harness session ids, `--continue` in a shared cwd
  can resume a peer's session, it is claude-only, and a resumed near-full session is still
  near-full.
- **Fixed sleeps only.** A sleep is a guess at turn end; a busy TUI queues Enter-submitted
  input, so the clear and the resume line could land mid-turn (challenge 4a340b35).
