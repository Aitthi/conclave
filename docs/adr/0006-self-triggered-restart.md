---
status: accepted
---

# Self-triggered restart: the agent wakes itself when its context nears full

Conclave already has a human-triggered Restart · resume (kill → respawn → resume-from-handoff)
and an engine-side auto-compact gate (`should_auto_compact`, 90%) that only works where the
engine can see a token meter — which CLI agents (Claude Code, Codex) don't expose: their
context state lives inside their own TUI. Result: a CLI agent near context exhaustion just
degrades until a human notices.

- **The agent is the trigger, not the engine.** A CLI agent SEES its own harness's context
  warnings; the engine would have to scrape PTY output per-harness to learn the same thing —
  brittle and format-coupled. So the agent runs `conclave restart` (no args = itself, resolved
  from `CONCLAVE_INSTANCE_ID`) when its harness warns that context is nearly full. The
  engine-side scraper was rejected for v1; it can arrive later as a safety net for agents that
  ignore the rule.
- **Memory travels via the existing handoff pipeline, fully automated.** Self-restart arms the
  same save-gated tail as the human-triggered flow: handoff saved → process killed → fresh
  terminal spawned → resume prompt injected. The alternative — respawning with the harness's
  own session persistence (`claude --resume`) and skipping snapshots entirely — was rejected:
  Conclave doesn't track harness session ids, in a shared workspace cwd `--continue` can
  resume a PEER's session, it's claude-only, and a resumed near-full session is still
  near-full. The agent-authored handoff is also richer than a harness auto-summary.
- **Self-trigger skips the save-prompt injection.** The human-triggered flow injects a "write
  your handoff" prompt because the agent doesn't know a restart is coming. A self-triggered
  agent already knows — `conclave restart` arms the tail and PRINTS the instruction (write
  handoff, then `conclave snapshot save …`) as its command output instead of injecting a turn
  into the agent's own mid-turn TUI. The save-gates-the-kill ordering is unchanged: an agent
  that never saves is never killed.
- **The standing rule lives in the Strategic Compact builtin skill** (the sidecar layer that
  survives `/clear`), not in a chat directive: when the harness warns context is near full,
  run `conclave restart`, then write and save the handoff immediately.
