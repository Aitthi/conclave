# Session Snapshots

Durable, agent-written handoff notes for the Conclave build. The point: when a
session is about to `/compact` or `/clear`, the auto-generated summary drops
load-bearing detail (exact SHAs, standing constraints, deferred tasks, the
gotcha you just fixed). A snapshot is the agent writing that detail down *on
purpose*, so a fresh-context agent resumes exactly where the last one stopped —
no guessing, no re-deriving.

This is the [strategic-compact](https://github.com/affaan-m/ECC/blob/main/.agents/skills/strategic-compact/SKILL.md)
idea made concrete for this repo: compact at logical boundaries, but write the
state to disk first so nothing important lives only in the conversation.

## Workflow

1. **Before `/compact` or `/clear`** (phase boundary, end of session, milestone):
   write a new snapshot file capturing the current state (template below).
2. Commit it (author `detoro <meanstack20@gmail.com>` per repo convention).
3. `/clear`.
4. **Fresh agent, first action:** read the newest file in `docs/snapshots/`
   (`ls -t docs/snapshots/*.md | head -1`) and resume from it.

## Naming

`YYYY-MM-DD-<short-topic>.md` — date-prefixed so `ls -t` / lexical sort both put
the newest last. Append `-2`, `-3` if more than one lands on a day.

## What a good snapshot contains

- **Standing constraints** — the rules that outlive any single task (git author,
  commit trailer, UI-copy language, secrets policy, DoD baselines). Copy them
  forward verbatim every snapshot; never assume the next agent inherits them.
- **Where we are** — current branch, last commit SHA, what just shipped.
- **Baselines** — the exact verify commands + their last-known-green status.
- **Deferred / pending** — offered-but-not-started work, gated on user go-ahead.
- **File map** — the load-bearing files for the active feature and what each does.
- **Gotchas** — false-positive hooks, fixed-the-hard-way bugs, non-obvious traps.
- **Resume hint** — the single most likely next action.

## Template

```markdown
---
date: YYYY-MM-DD
branch: <git branch>
head: <short SHA at write time>
status: <one-line state — e.g. "M6 shipped, awaiting runtime test">
---

# Snapshot: <topic>

## Standing constraints (carry forward verbatim)
- ...

## Where we are
- ...

## Baselines (last-known status)
- Rust: `cargo test --lib` · `cargo clippy --all-targets -- -D warnings` · `cargo fmt --check`
- Frontend: `pnpm exec tsc --noEmit` · `pnpm build`

## Recent commits (this session)
| SHA | summary |
| --- | --- |

## Deferred / pending (gated on user go-ahead)
- ...

## Load-bearing files
- `path` — what it does

## Gotchas
- ...

## Resume hint
- ...
```
