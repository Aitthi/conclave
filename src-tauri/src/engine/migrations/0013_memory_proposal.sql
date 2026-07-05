-- Review queue for transcript-distilled memory proposals (plan
-- memory-distill-queue). A proposal is a candidate memory mined from a
-- transcript; it becomes a `memory_chunk` only when a reviewer approves it, so
-- unproven auto-writes never poison the semantic-search commons. No embedding
-- is stored here: rejected junk must never cost an embed (embedding happens at
-- approve time, in `commands::memory::approve`).
--
-- `content_hash` is the same NFC SHA-256 used for `memory_chunk`, so a proposal
-- can be deduped against both the queue (this table's UNIQUE key) and the live
-- store before it is ever created. Rejected rows are KEPT: their content_hash
-- keeps the same fact from being re-proposed on the distiller's next run.
CREATE TABLE memory_proposal (
  id            TEXT PRIMARY KEY,
  workspace_id  TEXT NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
  proposer_id   TEXT NOT NULL,            -- workspace_agent id of the distiller
  text          TEXT NOT NULL,
  source_note   TEXT,                     -- e.g. "transcript 3f2a….jsonl 2026-07-04"
  content_hash  TEXT NOT NULL,            -- NFC SHA-256, same fn as memory_chunk
  state         TEXT NOT NULL DEFAULT 'pending'
                CHECK (state IN ('pending','approved','rejected')),
  reviewer_id   TEXT,                     -- workspace_agent id of the reviewer
  review_reason TEXT,
  chunk_id      TEXT,                     -- memory_chunk id, set on approve
  created_at    TEXT NOT NULL,
  reviewed_at   TEXT,
  UNIQUE (workspace_id, content_hash)
);
CREATE INDEX idx_memory_proposal_ws_state ON memory_proposal (workspace_id, state);
