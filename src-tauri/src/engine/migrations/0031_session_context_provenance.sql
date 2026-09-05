-- Context-gauge provenance (preflight correction b1140da9, D2).
--
-- runtime/transcript_context.rs already produces `observed_at` and
-- `source_kind` on every TranscriptContextReading, but
-- repo::session::set_context_reading discarded both and stamped
-- `last_active_at = now()` instead. The Overview contract requires the current
-- context to be "a separate latest gauge with source and observation time",
-- so the two values the reader already knows are persisted alongside the
-- tokens/limit they describe.
--
-- NULL on existing rows means "provenance was never observed" — old readings
-- are never relabelled as newly measured.

ALTER TABLE session ADD COLUMN context_source TEXT DEFAULT NULL;
ALTER TABLE session ADD COLUMN context_observed_at TEXT DEFAULT NULL;
