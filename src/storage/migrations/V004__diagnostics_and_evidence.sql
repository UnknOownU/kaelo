-- V004: Diagnostics, evidence, and page history tables

-- Track fetch evidence for citations and trust
CREATE TABLE IF NOT EXISTS fetch_evidence (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    strategy TEXT NOT NULL,
    retrieved_at TEXT NOT NULL,
    confidence TEXT NOT NULL,
    citations_json TEXT,
    diagnostics_json TEXT,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_fetch_evidence_url ON fetch_evidence(url);

-- Page version history for diff/watch
CREATE TABLE IF NOT EXISTS page_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    url TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content_body TEXT NOT NULL,
    strategy TEXT NOT NULL,
    fetched_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_page_history_url ON page_history(url);
CREATE INDEX IF NOT EXISTS idx_page_history_fetched_at ON page_history(fetched_at);

-- Add extraction_mode and diagnostics columns to url_cache
ALTER TABLE url_cache ADD COLUMN extraction_mode TEXT DEFAULT 'markdown';
ALTER TABLE url_cache ADD COLUMN diagnostics_json TEXT;
