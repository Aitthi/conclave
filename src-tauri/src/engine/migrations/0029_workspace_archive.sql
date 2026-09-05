ALTER TABLE workspace ADD COLUMN archived_at TEXT DEFAULT NULL;
CREATE INDEX idx_workspace_archived_at ON workspace(archived_at DESC, id);
