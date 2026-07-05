-- Promote `artifact` from a chat-message child to a first-class,
-- workspace-scoped store (plan design-artifact-store, Lane A). An artifact is
-- now any significant, self-contained agent output — a document, code file,
-- HTML page, SVG, diagram, or React component — written via
-- `conclave artifact add`, independent of any chat message.
--
-- The original table (0001_init.sql:110) hung every artifact off a
-- `message_id NOT NULL` and stored only a raw `html` blob with a `filename`.
-- To make `message_id` nullable AND add the new columns, SQLite forces a
-- table rebuild (it cannot ALTER a column to drop NOT NULL): create the new
-- shape, copy the old rows into it, drop the old, rename.
--
-- Existing chat-parsed rows are preserved verbatim: their `html` folds into
-- the new `content` column and they are tagged `kind = 'html'`, keeping
-- `workspace_id`/`agent_id`/`title` NULL (they were never workspace-scoped).
-- No inbound foreign keys point at `artifact`, so the drop/rename is safe
-- inside the migration transaction without toggling `PRAGMA foreign_keys`.
CREATE TABLE artifact_new (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES workspace(id) ON DELETE CASCADE,
    agent_id     TEXT,                                             -- creator agent instance/def id, free text
    message_id   TEXT REFERENCES message(id) ON DELETE CASCADE,    -- now nullable (chat-parsed rows only)
    title        TEXT,
    kind         TEXT,                                             -- markdown|code|html|svg|mermaid|react|text
    filename     TEXT,
    content      TEXT,                                             -- artifact body (old `html` folds in here)
    sandboxed    INTEGER,
    created_at   TEXT NOT NULL
);

INSERT INTO artifact_new
    (id, workspace_id, agent_id, message_id, title, kind, filename, content, sandboxed, created_at)
SELECT
    id, NULL, NULL, message_id, NULL, 'html', filename, html, sandboxed, created_at
FROM artifact;

DROP TABLE artifact;
ALTER TABLE artifact_new RENAME TO artifact;

CREATE INDEX idx_artifact_ws_created ON artifact (workspace_id, created_at DESC);
