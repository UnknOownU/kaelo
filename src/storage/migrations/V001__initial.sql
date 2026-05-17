CREATE TABLE IF NOT EXISTS domain_strategies (
    domain TEXT NOT NULL,
    strategy TEXT NOT NULL,
    avg_latency_ms INTEGER NOT NULL DEFAULT 0,
    success_rate REAL NOT NULL DEFAULT 0.0,
    total_requests INTEGER NOT NULL DEFAULT 0,
    last_success_at TEXT,
    last_check_at TEXT NOT NULL,
    extra_headers TEXT,
    PRIMARY KEY (domain, strategy)
);

CREATE TABLE IF NOT EXISTS url_cache (
    url TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    compressed_content BLOB,
    content_type TEXT,
    original_size INTEGER,
    created_at TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);
