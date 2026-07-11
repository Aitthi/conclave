-- M1 fix (design §7.1): make count_tokens failures diagnosable. The GUI app's
-- stderr → /dev/null, so a `count_failure` sample previously left no trace of the
-- real HTTP status/body. `error_snippet` durably records the count_tokens error
-- (HTTP status + ≤200-char JSON body snippet) on failure; NULL otherwise.
-- SQL-diagnosis only — deliberately NOT surfaced in checkpoint-report.
ALTER TABLE proxy_checkpoint_metric
    ADD COLUMN error_snippet TEXT;
