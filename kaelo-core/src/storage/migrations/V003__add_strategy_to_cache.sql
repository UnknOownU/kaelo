-- Change url_cache PRIMARY KEY from (url) to (url, strategy).
-- SQLite cannot ALTER PRIMARY KEY, so we recreate the table.

CREATE TABLE url_cache_new (
    url TEXT NOT NULL,
    strategy TEXT NOT NULL DEFAULT 'HttpSimple',
    content_hash TEXT NOT NULL,
    compressed_content BLOB,
    content_type TEXT,
    original_size INTEGER,
    created_at TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT,
    PRIMARY KEY (url, strategy)
);

INSERT INTO url_cache_new (url, strategy, content_hash, compressed_content, content_type,
    original_size, created_at, last_accessed_at, expires_at, etag, last_modified)
SELECT url, 'HttpSimple', content_hash, compressed_content, content_type,
    original_size, created_at, last_accessed_at, expires_at, etag, last_modified
FROM url_cache;

DROP TABLE url_cache;
ALTER TABLE url_cache_new RENAME TO url_cache;
