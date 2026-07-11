CREATE TABLE IF NOT EXISTS proxy_checkpoint_metric (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    model TEXT NOT NULL,
    earliest_changed_byte INTEGER NOT NULL,
    earliest_changed_msg INTEGER NOT NULL,
    r_tokens INTEGER NOT NULL,
    gross_candidate_tokens INTEGER NOT NULL,
    stub_overhead_tokens INTEGER NOT NULL,
    s_net_tokens INTEGER NOT NULL,
    q REAL NOT NULL,
    projected_break_even REAL NOT NULL,
    projected_post_tokens INTEGER NOT NULL,
    plateau_turns INTEGER NOT NULL,
    non_recoverable_kept_tokens INTEGER NOT NULL,
    provider_estimate INTEGER NOT NULL,
    count_failure INTEGER NOT NULL,
    method_version TEXT NOT NULL,
    bytes_est_tokens INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_proxy_checkpoint_metric_created_at
    ON proxy_checkpoint_metric(created_at);
