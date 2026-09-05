-- Durable Claude usage reconciliation evidence (review a12f77f2 C6, plan
-- docs/plans/2026-09-05-usage-reconciliation-schema.md).
--
-- Every row of one Claude request group must agree on response identity,
-- model, stop reason and each usage component across restart and arbitrary
-- replay. A bounded in-memory cache of recent groups cannot prove agreement
-- after eviction, so the two scalars the existing columns cannot recover are
-- persisted on the event itself:
--
-- * stop_reason — the source's terminal stop reason, bounded to 128
--   characters. An oversized value is never truncated into apparent
--   agreement: the collector stores NULL with a diagnostic instead.
-- * source_uncached_input_tokens — the source's own uncached input counter.
--   input_tokens is the cache-inclusive SUM; when a cache component was
--   missing the sum alone cannot recover this value, so it is kept as
--   reported, under the same 2^40 ceiling as every other counter.
--
-- Both are NULL for pre-migration rows and for non-Claude sources. A legacy
-- Claude row with NULL stop_reason is incomplete evidence: a replay against it
-- stays conservatively conflicting rather than proving agreement. Additive
-- only; 0030 and 0031 are untouched. Never raw transcript JSON or text.

ALTER TABLE model_usage_event ADD COLUMN stop_reason TEXT DEFAULT NULL
    CHECK (stop_reason IS NULL OR length(stop_reason) <= 128);
ALTER TABLE model_usage_event ADD COLUMN source_uncached_input_tokens INTEGER DEFAULT NULL
    CHECK (source_uncached_input_tokens IS NULL
           OR source_uncached_input_tokens BETWEEN 0 AND 1099511627776);
