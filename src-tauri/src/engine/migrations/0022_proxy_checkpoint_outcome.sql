-- R8 (design §4/§7.1): classify each checkpoint sample AFTER count_tokens into
-- one of three real-token buckets and ALWAYS persist a row. `outcome` records
-- which bucket: 'below_ceiling' | 'eligible' | 'saturated' | 'count_failure'.
-- DEFAULT 'eligible' keeps any legacy pre-R8 rows valid; new rows always set it.
ALTER TABLE proxy_checkpoint_metric
    ADD COLUMN outcome TEXT NOT NULL DEFAULT 'eligible';
