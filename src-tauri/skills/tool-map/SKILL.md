---
name: Tool Map
description: One-screen map of which conclave verb family to use for what — work items on tasks, worktrees on lanes, ad-hoc facts on the blackboard, knowledge in memory, messages via tell. Full syntax lives in `conclave help`.
mandatory: true
---

Work items ride tasks, never bb keys. `task list` is slim orientation, `task
brief <ws> <slug>` the bounded resume packet, `task get` the full record —
deepest only when the lighter one lacks what you need. The blackboard holds
durable facts that fit no task; Memory holds knowledge that outlives it. This
indexes which family owns what; **`conclave help` is the live, full flag-by-flag
reference — trust it over any cached copy** (including this one).

Any CLI response over 10KB is capped — the first 2KB prints and the full text
lands in the app-support `cli-output/` dir (the printed pointer names the file);
`CONCLAVE_NO_CAP=1` disables it, and the `snapshot` family is never capped. `task
gate` likewise shows only a bounded log excerpt — the full log lands in
`cli-output`, its path on the gate event as `logPath`.

- **Work items** — `task list · brief · get · create · claim · state · note · gate · challenge · rule · close · watch/unwatch · plan-check`; plus `uishot [--task <slug>] <args>` to run the workspace UI capture and SEE the result. `task list` is slim open-task rows by default (`--full` = plan-bearing rows, `--all` = slim shape + merged/abandoned). All work state (claims, plans, gates, challenges) lives here, evented.
- **Artifacts** — `artifact add · list · get`: persist a significant, self-contained output (see below); your agent id is stamped automatically.
- **Lanes** — `lane start` (claim + worktree in one step), `lane finish` (integrator teardown after merge), `lane guard install` (shared-checkout commit-scope guard).
- **Stage** — `stage status · diff · commit · snap · log · restore · clear`.
- **Peers** — `agent list` (roster: ids, roles, skills, model, `working` flag), `tell <id>` (the ONLY channel that reaches a peer), `msg list/all`, `send`, `run`.
- **Workspaces** — `ws list · use`.
- **Blackboard** — `bb list · get · set · delete`. Key `config:distill-auto` opts a workspace into the timer-driven distiller nudge (`{distiller, reviewer, cooldownHours}`); absent = OFF, `bb delete` is the kill switch. Keys `config:stall-minutes` / `config:stall-cooldown-minutes` override the stall-alert threshold/cooldown per workspace (defaults 10/30, clamped 5–240/10–1440).
- **Memory** — `memory search · remember · delete · status · propose · queue · approve · reject`.
- **Orient** — `orient <ws>`: ONE bounded fresh-context packet — slim live tasks, roster, your latest messages, blackboard heads, your watches, self — the first command after any restore/clear, replacing the old task-list + agent-list + msg-list + bb-list fan-out.
- **Context** — `snapshot save · last · list · read · create`; `restart` (self-triggered save-then-die).
- **Browser** — `browser open · goto · status · snapshot · click · type · eval · screenshot · close`: drive a page without Playwright/Puppeteer.
- **Code intel** — `code stats · files · tree · symbols · find · callers · callees · refs · impact · rename · rewrite`: tree-sitter survey/cross-reference/AST-edit of a checkout (11 languages, engine-cached index; defaults to your cwd, `--path <DIR>` overrides). PREFER these over grep/find for symbol questions. `rename`/`rewrite` are dry-run by default — `--apply` writes and refreshes the cache; `needs_anchor`/`pattern-compile` come back as exit-0 JSON, read the payload.

## Council plan contract (conclave-plan:v1)

Council-planned tasks open with a ten-line execution header (title line 1,
`<!-- conclave-plan:v1` JSON lines 2-10 carrying owner, authority, council,
planPath, baseSha, escalation, readingOrder, boundary, consumes, produces,
gates). The header stored at `task create` is IMMUTABLE and stays byte-for-byte
equal to the first ten lines of the repo plan named by its `planPath`; only
prose below line 10 may be amended in place. `task plan-check` prints the
precise validation error when a header is malformed — trust its output over any
copied schema.

- `task create <ws> <slug> <title...> --plan-file <p> --watchers <id,id>` —
  subscribes the owner plus every listed workspace agent to the task in the SAME
  transaction as creation; an unknown or cross-workspace id fails the whole
  create.
- `task plan-check <ws> <slug>` — validates the canonical plan in YOUR current
  checkout against the stored header (fields, boundary set-equality, anchors,
  required sections, UI canon rule) and SHA-256-fingerprints its exact bytes.
  On success it appends a typed `plan_check` event; run it wrapped as
  `task gate <ws> <slug> -- conclave task plan-check <ws> <slug>` so the ledger
  carries BOTH the typed fingerprint event and ordinary gate evidence.
- Claiming a council-tagged task (direct `task claim` or `lane start`) runs a
  CLI-local freshness preflight: it warns loudly on stderr when the plan-check
  event is missing, unverifiable, or the plan/header changed since the last
  green check — but v1 NEVER blocks the claim; the claim wire is unchanged.
  Lane start hashes the plan inside the freshly created worktree, so what is
  checked is exactly what you will read.

Prefer `stage commit` over raw `git add`/`git commit` in the shared checkout:
`stage commit` writes ONLY the task's boundary paths through a private index —
the shared `.git/index` is never touched and attribution stays native git
authorship. Raw git risks sweeping a peer's staged work (b9ab709) and lands the
commit under the shared human identity (c3d8fcb).

## Artifacts — when to save one

Save an artifact when an output is **significant and self-contained**: >~15
lines, likely to be edited/reused outside this conversation, standing on its own,
something you or the human will refer back to. Kinds: `markdown · code · html ·
svg · mermaid · react`, `text` otherwise. Do NOT wrap a throwaway snippet, a
one-line answer, or a value that only makes sense mid-conversation — those go in
chat, a task note, or the blackboard. One artifact = one coherent deliverable;
add a new version rather than cram several into one.
