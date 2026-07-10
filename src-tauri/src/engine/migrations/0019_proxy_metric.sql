CREATE TABLE IF NOT EXISTS proxy_request_metric (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    model TEXT NOT NULL,
    mode TEXT NOT NULL,
    decision TEXT NOT NULL,
    request_bytes_in INTEGER NOT NULL,
    request_bytes_out INTEGER NOT NULL,
    elisions INTEGER NOT NULL,
    bytes_saved INTEGER NOT NULL,
    input_tokens INTEGER,
    cache_read_tokens INTEGER,
    cache_creation_tokens INTEGER,
    output_tokens INTEGER
);

CREATE INDEX IF NOT EXISTS idx_proxy_request_metric_created_at
    ON proxy_request_metric(created_at);
