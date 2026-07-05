# Plan: skill prose pass — delegation load-balance rule + two deferred Lows

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `skill-prose-pass` · Implementer: Dabin eecebcbe · Reviewer: Mellow (LAND, blocking)
Status: APPROVED by human 2026-07-05 ("weight ดีๆ เพราะงานซ้อนกันแล้วมันรอ แต่สองตัวนี้ว่าง ต้องแก้ SKILL" — the balance rule must live in the skill layer, not just memory/chat).

## Why

Delegation this session routed every lane to the same two familiar
implementers while two capable agents (codex) sat idle — work queued behind
busy agents. A directive sent as a message or memory dies or hides at the
moment of decision; rules that must HOLD go in the skill files agents re-read
every fresh context (the leadership skill says this about itself). Two small
deferred Lows in the implementer prose ride along.

## Task A — leadership: load-balance rule

`src-tauri/skills/leadership/SKILL.md`, in the roster/working-flag cluster
(currently ~lines 207-213, after the `model`/`cliKind` bullet). Insert one
bullet, this text verbatim (adjust indentation to neighbors):

- Weight new lanes by AVAILABILITY, not familiarity: when independent work
  exists and a capable agent sits idle, assigning it to an already-busy
  favorite queues the workspace behind one context window. Familiarity is a
  tiebreaker between two IDLE agents, not a reason to wait. Routing every
  lane to the same two implementers also concentrates codebase knowledge —
  the idle agent never becoming reliable is a cost you chose, not a fact you
  discovered.

## Task B — implementer: two deferred Lows

`src-tauri/skills/implementer/SKILL.md`:

1. COMMIT FIRST, THEN GATE — in the gate-through-the-ledger bullet
   (~line 126): add one sentence: "Commit BEFORE gating: `task gate` pins
   `git rev-parse HEAD` at run time, so gating uncommitted work records the
   parent commit as evidence — a SHA the reviewer cannot check your work out
   at." (Reason on record: Mellow's finding, lane tool-map, memory 1aa2aa10
   ships the close-verb half.)
2. close-vs-merged dedup (~line 157): the prose currently reads as if
   implementers end tasks with `task close`. Rewrite that sentence to:
   "Move YOUR work to `review` (`conclave task state <ws> <slug> review`);
   `merged` is the integrator's move after the merge lands. `task close` is
   the integrator's shortcut, not the implementer's exit."

## Boundary

`src-tauri/skills/leadership/SKILL.md`,
`src-tauri/skills/implementer/SKILL.md`. Nothing else.
NOT `src-tauri/skills/tool-map/SKILL.md` — owned by lane stage-v1 (Dew)
right now; its commit-first-then-gate row is DEFERRED to after stage-v1
merges (remains a recorded Low).

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full — covers builtin-skill frontmatter parsing) ·
  `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND (blocking): inserted prose matches this plan verbatim in
  meaning, placement doesn't orphan neighboring bullets, frontmatter intact.

## Risk ledger

- Reaches live agent sidecars only after next rebuild+install (sidecars
  regenerate from the installed bundle's Resources/skills).
- Keep edits surgical — these files are the live behavior spec for every
  agent; a broken sentence ships to everyone at once.
- Markdown only; if anything seems to require code changes, escalate — that
  means the plan is wrong.
