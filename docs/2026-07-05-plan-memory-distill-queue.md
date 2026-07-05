# Plan: memory distiller with review queue — transcripts → proposed memories → lead-gated store

Date: 2026-07-05 · Owner: Detoro bfb737ff · authority: in-loop
Task: `memory-distill-queue` · Implementer: Tiësto fd0dec79 · Reviewer: Mellow (LAND, blocking)
Status: DRAFT — awaiting human approval before task creation.

## Why

Memory capture today depends entirely on each agent's discipline: a lesson
never saved with `memory remember` before a context clear is lost. Transcripts
(Claude Code JSONL) are the only ground-truth record of what actually happened
— including failed approaches, the most valuable and least-saved memory
category. The human approved a distiller that mines transcripts into memory,
**gated by a review queue** (human ruling 2026-07-05): junk that reaches the
store poisons every future search (hygiene commons), so nothing auto-written
lands directly.

Recorded constraint honored: the MemPalace benchmark report
(`docs/2026-07-05-memory-benchmark-mempalace.md` §52, §267) rejects *verbatim
auto-ingest*. This design distills (LLM-curated single facts) and gates
(review queue) — it is NOT verbatim ingest; the rejection stands untouched.

## Decisions (settled, encode exactly these)

1. **Review queue is a first-class engine object**: new table
   `memory_proposal`; proposals are embedded and copied into `memory_chunk`
   only on approve. No embedding happens at propose time (rejected junk never
   costs an embed).
2. **Distiller is an agent + skill, not engine code**: the app has an
   embedding model but no generative LLM. Distillation = a builtin skill
   (`src-tauri/skills/memory-distiller/SKILL.md`) any assigned agent runs.
3. **Trigger v1 is on-demand** (lead or human asks an agent to run the
   skill). Auto-trigger (task_timer nudge / on task close) is Phase 2, only
   after pilot precision is proven. Rationale: a background writer with
   unproven precision floods the queue and burns review time.
4. **Approved chunks get `source_kind='distilled'`**, `source_id` = the
   proposer's workspace_agent id. New kind (vs reusing `'agent'`) so distilled
   chunks are greppable and bulk-purgeable if the pilot sours.
5. **Proposer cannot approve their own proposal** — enforced engine-side by
   agent-id comparison. This is the one mechanism (vs convention) because the
   gate IS the feature.
6. **Source scope v1**: Claude Code JSONL under the workspace's single
   project dir (`~/.claude/projects/-Users-detoro-code-codeup/`, 86 files
   today). Codex agents (`~/.codex/sessions/`) deferred — different format,
   two of seven agents. Handoff snapshots (`snapshot.carried_forward`) are an
   allowed secondary source (already agent-authored summaries).

## Task 1 — engine: proposal queue (migration + repo + commands + CLI)

**Migration** `src-tauri/src/engine/migrations/0013_memory_proposal.sql`
(+ the `if version < 13` block in `db.rs::migrate`, + update
`migrate_creates_all_tables` test at db.rs:213):

```sql
CREATE TABLE memory_proposal (
  id            TEXT PRIMARY KEY,
  workspace_id  TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  proposer_id   TEXT NOT NULL,            -- workspace_agent id
  text          TEXT NOT NULL,
  source_note   TEXT,                     -- e.g. "transcript 3f2a….jsonl 2026-07-04"
  content_hash  TEXT NOT NULL,            -- NFC SHA-256, same fn as memory_chunk
  state         TEXT NOT NULL DEFAULT 'pending'
                CHECK (state IN ('pending','approved','rejected')),
  reviewer_id   TEXT,
  review_reason TEXT,
  chunk_id      TEXT,                     -- memory_chunk id, set on approve
  created_at    TEXT NOT NULL,
  reviewed_at   TEXT,
  UNIQUE (workspace_id, content_hash)
);
CREATE INDEX idx_memory_proposal_ws_state ON memory_proposal (workspace_id, state);
```

**Repo**: new `src-tauri/src/engine/repo/memory_proposal.rs` following the
chain-builder idioms of `repo/memory.rs` (create, list_by_state, get,
set_reviewed).

**Commands** (in `src-tauri/src/engine/commands/memory.rs`, router routes
`memory.propose`, `memory.queue`, `memory.approve`, `memory.reject`):

- `propose`: validates proposer is a workspace_agent of the workspace (reuse
  `validate_source` shape, memory.rs:329). Dedup TWICE at propose time:
  against `memory_proposal` (unique key) AND against `memory_chunk`
  content_hash — an already-stored fact returns `{deduped: true}` and creates
  nothing.
- `approve`: pending only; reviewer ≠ proposer (error otherwise); embeds via
  the existing `embed_one` path and upserts through `repo::memory::upsert_chunk`
  with `source_kind='distilled'`, `source_id=proposer_id`; stamps
  `chunk_id/reviewer_id/reviewed_at`, state → approved. Extend
  `validate_source`/kind handling for `'distilled'`.
- `reject`: pending only; stamps reviewer/reason/reviewed_at, state → rejected.
  Rejected rows are KEPT (they teach the distiller's next run what not to
  re-propose — content_hash uniqueness blocks re-proposal automatically).
- `queue`: list, `--state` filter, default pending, newest first.

**CLI** (`src-tauri/src/bin/conclave-cli.rs`): four verbs under the memory
family, usage lines beside memory's existing ones (:81-84), self-id stamping
for `propose`/`approve`/`reject` via `expand_self_args` (:227) like
`remember`. Update the memory rows of `src-tauri/skills/tool-map/SKILL.md`.

## Task 2 — distiller skill (same lane, after Task 1 lands)

New builtin skill `src-tauri/skills/memory-distiller/SKILL.md` instructing
the running agent to:

1. Read high-water mark from bb key `note:distill-hwm` (ISO timestamp;
   absent → default to 48h ago, NEVER the full 86-file history).
2. List `~/.claude/projects/-Users-detoro-code-codeup/*.jsonl` with mtime >
   hwm; for each, extract conversation text with a small python/jq pass —
   never read whole JSONL files into context.
3. Distill ONLY the categories the Memory skill allows: failed approaches +
   why, environment/tooling quirks, exact working incantations, decision
   reasoning absent from repo records. Explicitly skip: anything already in
   git/task ledger/docs, status churn, and ALL secrets (redact; a proposal
   containing a token is a review-time reject).
4. For each candidate: `conclave memory search` first (hybrid ranker, live);
   only propose if no existing chunk covers it. Then
   `conclave memory propose <ws> <text> --source-note <file+date>`.
5. Update `note:distill-hwm` to the run's start time; post a one-line summary
   (N candidates, M proposed, K deduped) to the requester.

Review protocol (documented in the skill + this plan): lead runs
`memory queue`, then `approve`/`reject` each with reasoning. Precision target
for the pilot: if <50% of proposals are approve-worthy across the first two
runs, stop and rethink before Phase 2.

## Tests (mirror `mod tests` patterns in commands/memory.rs:1444+)

1. Propose→approve happy path: chunk exists with `source_kind='distilled'`,
   proposal carries chunk_id, searchable afterwards.
2. Self-approve rejected with a clear error.
3. Approve/reject on non-pending → error; reject stamps reason.
4. Propose dedup: identical text vs existing chunk AND vs existing proposal
   both return deduped, create nothing.
5. Migration test updated; router route-name tests for the four verbs.

## Boundary

`src-tauri/src/engine/migrations/0013_memory_proposal.sql`,
`src-tauri/src/engine/db.rs`, `src-tauri/src/engine/repo/memory_proposal.rs`
(new), `src-tauri/src/engine/repo/mod.rs`,
`src-tauri/src/engine/commands/memory.rs`,
`src-tauri/src/bin/conclave-cli.rs`, `src-tauri/skills/tool-map/SKILL.md`,
`src-tauri/skills/memory-distiller/SKILL.md` (new). Nothing else.

## Gates (commit first, then gate; from src-tauri)

- `cargo test` (full) · `cargo clippy --all-targets -- -D warnings`.
- Mellow LAND review before merge (blocking): schema/state-machine
  correctness, self-approve enforcement, dedup-both-ways, `'distilled'`
  source_kind purgeability, skill text matches CLI behavior.

## Risk ledger

- Everything here reaches live agents only after the next rebuild+install.
- Transcripts may contain secrets — the skill mandates redaction AND the
  queue is the backstop; reviewers reject on sight.
- Transcript JSONL files are large; context discipline (extract, don't read
  raw) is stated in the skill, but a careless distiller can still blow its
  context — first pilot run should be watched.
- The single-project-dir assumption holds today (verified: lane worktree
  sessions produced no separate project dirs) but is environmental, not
  guaranteed — the skill globs the dir it's told, and Phase 2 can widen.
- Do NOT populate the dormant `message` table (0001_init.sql:57) as a
  transcript store — out of scope; JSONL is the source of truth.
- Phase 2 (auto-trigger via task_timer.rs tick or task-close hook at
  task.rs:705) is DEFERRED and needs its own plan + human go-ahead.
