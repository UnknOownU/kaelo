use std::io::{Read as _, Write as _};
use std::time::Duration;

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

use super::Storage;

const MAX_ENTRY_SIZE: usize = 100 * 1024; // 100 KB

/// Cached ETag / Last-Modified for conditional requests.
#[derive(Debug, Clone)]
pub struct ConditionalHeaders {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CachedContent {
    pub content: String,
    pub content_hash: String,
    pub content_type: String,
    pub original_size: usize,
    pub age_seconds: u64,
    pub from_cache: bool,
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: u64,
    pub total_size_bytes: u64,
    pub top_domains: Vec<(String, u64)>,
}

fn gzip_compress(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .context("failed to write to gzip encoder")?;
    encoder
        .finish()
        .context("failed to finish gzip compression")
}

fn gzip_decompress(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = GzDecoder::new(data);
    let mut buf = Vec::new();
    decoder
        .read_to_end(&mut buf)
        .context("failed to decompress gzip data")?;
    Ok(buf)
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Expects URLs like `https://example.com/path` → `example.com`.
fn extract_domain(url: &str) -> Option<String> {
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = stripped.split('/').next()?;
    // Strip port if present
    let host = host.split(':').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn now_iso() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;
    secs.to_string()
}

fn epoch_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn epoch_from_iso(s: &str) -> u64 {
    s.parse::<u64>().unwrap_or(0)
}

pub struct ContentCache<'a> {
    storage: &'a Storage,
}

impl<'a> ContentCache<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub fn put(&self, url: &str, content: &str, content_type: &str, ttl: Duration) -> Result<()> {
        self.put_with_conditional(url, content, content_type, ttl, None)
    }

    pub fn put_with_conditional(
        &self,
        url: &str,
        content: &str,
        content_type: &str,
        ttl: Duration,
        conditional: Option<&ConditionalHeaders>,
    ) -> Result<()> {
        let original_size = content.len();
        if original_size > MAX_ENTRY_SIZE {
            return Ok(());
        }

        let content_hash = sha256_hex(content.as_bytes());
        let compressed = gzip_compress(content.as_bytes())?;

        let now = now_iso();
        let expires_at = (epoch_now_secs() + ttl.as_secs()).to_string();

        let etag = conditional.and_then(|c| c.etag.clone());
        let last_modified = conditional.and_then(|c| c.last_modified.clone());

        self.storage
            .conn()
            .execute(
                "INSERT OR REPLACE INTO url_cache
                (url, content_hash, compressed_content, content_type, original_size,
                 created_at, last_accessed_at, expires_at, etag, last_modified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    url,
                    content_hash,
                    compressed,
                    content_type,
                    original_size as i64,
                    now,
                    now,
                    expires_at,
                    etag,
                    last_modified,
                ],
            )
            .context("failed to insert into url_cache")?;

        Ok(())
    }

    /// Returns `None` if not found or expired (TTL elapsed).
    pub fn get(&self, url: &str) -> Result<Option<CachedContent>> {
        let conn = self.storage.conn();
        let mut stmt = conn.prepare(
            "SELECT content_hash, compressed_content, content_type, original_size,
                    created_at, expires_at
             FROM url_cache WHERE url = ?1",
        )?;

        let row_result = stmt.query_row([url], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        });

        let (content_hash, compressed, content_type, original_size, created_at, expires_at) =
            match row_result {
                Ok(r) => r,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
                Err(e) => return Err(e).context("failed to query url_cache"),
            };

        let now_secs = epoch_now_secs();
        let expires_secs = epoch_from_iso(&expires_at);
        if now_secs >= expires_secs {
            let _ = self.invalidate(url);
            return Ok(None);
        }

        let decompressed = gzip_decompress(&compressed)?;
        let content =
            String::from_utf8(decompressed).context("cached content is not valid UTF-8")?;

        // Touch last_accessed_at (LRU)
        conn.execute(
            "UPDATE url_cache SET last_accessed_at = ?1 WHERE url = ?2",
            rusqlite::params![now_iso(), url],
        )?;

        let created_secs = epoch_from_iso(&created_at);
        let age_seconds = now_secs.saturating_sub(created_secs);

        Ok(Some(CachedContent {
            content,
            content_hash,
            content_type,
            original_size: original_size as usize,
            age_seconds,
            from_cache: true,
        }))
    }

    pub fn invalidate(&self, url: &str) -> Result<()> {
        self.storage
            .conn()
            .execute("DELETE FROM url_cache WHERE url = ?1", [url])
            .context("failed to delete from url_cache")?;
        Ok(())
    }

    pub fn clear_domain(&self, domain: &str) -> Result<u64> {
        let conn = self.storage.conn();
        let http_prefix = format!("http://{domain}%");
        let https_prefix = format!("https://{domain}%");

        let count1 = conn
            .execute("DELETE FROM url_cache WHERE url LIKE ?1", [&http_prefix])
            .context("failed to clear domain (http)")?;

        let count2 = conn
            .execute("DELETE FROM url_cache WHERE url LIKE ?1", [&https_prefix])
            .context("failed to clear domain (https)")?;

        Ok((count1 + count2) as u64)
    }

    pub fn clear_older_than(&self, duration: Duration) -> Result<u64> {
        let cutoff = epoch_now_secs()
            .saturating_sub(duration.as_secs())
            .to_string();
        let count = self
            .storage
            .conn()
            .execute("DELETE FROM url_cache WHERE created_at < ?1", [&cutoff])
            .context("failed to clear old entries")?;
        Ok(count as u64)
    }

    pub fn clear_all(&self) -> Result<()> {
        self.storage
            .conn()
            .execute("DELETE FROM url_cache", [])
            .context("failed to clear url_cache")?;
        Ok(())
    }

    pub fn evict_lru(&self, max_size_bytes: u64) -> Result<u64> {
        let conn = self.storage.conn();

        let total_size: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(compressed_content)), 0) FROM url_cache",
                [],
                |row| row.get::<_, i64>(0),
            )
            .context("failed to compute cache size")?;

        if (total_size as u64) <= max_size_bytes {
            return Ok(0);
        }

        let mut stmt = conn.prepare(
            "SELECT url, LENGTH(compressed_content) FROM url_cache ORDER BY last_accessed_at ASC",
        )?;

        let entries: Vec<(String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut evicted: u64 = 0;
        let mut freed: i64 = 0;
        let target_free = (total_size as u64) - max_size_bytes;

        for (url, size) in entries {
            if (freed as u64) >= target_free {
                break;
            }
            conn.execute("DELETE FROM url_cache WHERE url = ?1", [&url])?;
            freed += size;
            evicted += 1;
        }

        Ok(evicted)
    }

    pub fn stats(&self) -> Result<CacheStats> {
        let conn = self.storage.conn();

        let entry_count: u64 = conn
            .query_row("SELECT COUNT(*) FROM url_cache", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|c| c as u64)?;

        let total_size_bytes: u64 = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(compressed_content)), 0) FROM url_cache",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|s| s as u64)?;

        let mut stmt = conn.prepare("SELECT url FROM url_cache")?;
        let urls: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut domain_counts: std::collections::HashMap<String, u64> =
            std::collections::HashMap::new();
        for url in &urls {
            if let Some(domain) = extract_domain(url) {
                *domain_counts.entry(domain).or_insert(0) += 1;
            }
        }

        let mut top_domains: Vec<(String, u64)> = domain_counts.into_iter().collect();
        top_domains.sort_by(|a, b| b.1.cmp(&a.1));
        top_domains.truncate(5);

        Ok(CacheStats {
            entry_count,
            total_size_bytes,
            top_domains,
        })
    }

    pub fn content_hash(&self, url: &str) -> Result<Option<String>> {
        let result = self.storage.conn().query_row(
            "SELECT content_hash FROM url_cache WHERE url = ?1",
            [url],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(hash) => Ok(Some(hash)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e).context("failed to query content hash"),
        }
    }

    pub fn get_conditional_headers(&self, url: &str) -> Result<ConditionalHeaders> {
        let result = self.storage.conn().query_row(
            "SELECT etag, last_modified FROM url_cache WHERE url = ?1",
            [url],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        );

        match result {
            Ok((etag, last_modified)) => Ok(ConditionalHeaders {
                etag,
                last_modified,
            }),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(ConditionalHeaders {
                etag: None,
                last_modified: None,
            }),
            Err(e) => Err(e).context("failed to query conditional headers"),
        }
    }

    pub fn content_hash_unchanged(&self, url: &str, new_content: &str) -> Result<bool> {
        let new_hash = sha256_hex(new_content.as_bytes());
        match self.content_hash(url)? {
            Some(cached_hash) => Ok(cached_hash == new_hash),
            None => Ok(false),
        }
    }

    pub fn find_by_hash(&self, hash: &str) -> Result<Vec<String>> {
        let conn = self.storage.conn();
        let mut stmt = conn.prepare("SELECT url FROM url_cache WHERE content_hash = ?1")?;

        let urls: Vec<String> = stmt
            .query_map([hash], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(urls)
    }

    pub fn dedup(&self) -> Result<u64> {
        let conn = self.storage.conn();

        let mut stmt = conn.prepare(
            "SELECT content_hash, COUNT(*) as cnt
             FROM url_cache
             GROUP BY content_hash
             HAVING cnt > 1",
        )?;

        let duplicates: Vec<(String, u32)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut removed: u64 = 0;
        for (hash, _count) in &duplicates {
            // Keep the most recently accessed entry, delete the rest.
            let mut stmt2 = conn.prepare(
                "SELECT url FROM url_cache
                 WHERE content_hash = ?1
                 ORDER BY last_accessed_at ASC",
            )?;
            let urls: Vec<String> = stmt2
                .query_map([hash], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;

            let to_delete = urls.len().saturating_sub(1);
            for url in urls.iter().take(to_delete) {
                conn.execute("DELETE FROM url_cache WHERE url = ?1", [url])?;
                removed += 1;
            }
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    fn make_cache() -> ContentCache<'static> {
        // We leak the Storage to get a 'static reference for testing.
        // This is fine for tests with in-memory DBs.
        let storage = Box::leak(Box::new(Storage::open(":memory:").unwrap()));
        ContentCache::new(storage)
    }

    #[test]
    fn test_put_and_get() {
        let cache = make_cache();

        let html = "<html><body>Hello, world!</body></html>";
        cache
            .put(
                "https://example.com/page1",
                html,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();

        let result = cache.get("https://example.com/page1").unwrap();
        assert!(result.is_some(), "should find cached entry");

        let cached = result.unwrap();
        assert_eq!(cached.content, html);
        assert_eq!(cached.content_type, "text/html");
        assert_eq!(cached.original_size, html.len());
        assert!(cached.from_cache);
    }

    #[test]
    fn test_hash_matches() {
        let cache = make_cache();

        let html = "<html><body>hash test</body></html>";
        cache
            .put(
                "https://example.com/hash",
                html,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();

        let expected_hash = sha256_hex(html.as_bytes());

        let cached = cache.get("https://example.com/hash").unwrap().unwrap();
        assert_eq!(cached.content_hash, expected_hash);

        let hash = cache.content_hash("https://example.com/hash").unwrap();
        assert_eq!(hash, Some(expected_hash));
    }

    #[test]
    fn test_ttl_expiry() {
        let cache = make_cache();

        cache
            .put(
                "https://example.com/ttl",
                "short-lived",
                "text/plain",
                Duration::from_secs(1),
            )
            .unwrap();

        assert!(cache.get("https://example.com/ttl").unwrap().is_some());

        thread::sleep(Duration::from_millis(1100));

        assert!(cache.get("https://example.com/ttl").unwrap().is_none());
    }

    #[test]
    fn test_gzip_compression() {
        let cache = make_cache();

        let html = "<html><body>".to_string()
            + &"Lorem ipsum dolor sit amet. ".repeat(500)
            + "</body></html>";

        cache
            .put(
                "https://example.com/compress",
                &html,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();

        let conn = cache.storage.conn();
        let compressed_size: i64 = conn
            .query_row(
                "SELECT LENGTH(compressed_content) FROM url_cache WHERE url = ?1",
                ["https://example.com/compress"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        assert!(
            (compressed_size as usize) < html.len(),
            "compressed size ({}) should be less than original ({})",
            compressed_size,
            html.len(),
        );

        let cached = cache.get("https://example.com/compress").unwrap().unwrap();
        assert_eq!(cached.content, html);
    }

    #[test]
    fn test_lru_eviction() {
        let cache = make_cache();

        // Insert 10 entries, each ~100 bytes compressed
        for i in 0..10 {
            let content = format!("entry-{i:04} {}", "x".repeat(50));
            cache
                .put(
                    &format!("https://example.com/page/{i}"),
                    &content,
                    "text/html",
                    Duration::from_secs(3600),
                )
                .unwrap();
        }

        let stats_before = cache.stats().unwrap();
        assert_eq!(stats_before.entry_count, 10);

        let half_max = stats_before.total_size_bytes / 2;
        let evicted = cache.evict_lru(half_max).unwrap();
        assert!(
            evicted >= 3,
            "should evict at least 3 entries, got {evicted}"
        );

        let stats_after = cache.stats().unwrap();
        assert!(stats_after.entry_count < 10);

        assert!(cache.get("https://example.com/page/0").unwrap().is_none());
        assert!(cache.get("https://example.com/page/1").unwrap().is_none());
    }

    #[test]
    fn test_invalidate() {
        let cache = make_cache();

        cache
            .put(
                "https://example.com/inv",
                "to-be-removed",
                "text/plain",
                Duration::from_secs(3600),
            )
            .unwrap();

        assert!(cache.get("https://example.com/inv").unwrap().is_some());

        cache.invalidate("https://example.com/inv").unwrap();
        assert!(cache.get("https://example.com/inv").unwrap().is_none());
    }

    #[test]
    fn test_clear_all() {
        let cache = make_cache();

        for i in 0..3 {
            cache
                .put(
                    &format!("https://example.com/clear/{i}"),
                    &format!("content-{i}"),
                    "text/plain",
                    Duration::from_secs(3600),
                )
                .unwrap();
        }

        assert_eq!(cache.stats().unwrap().entry_count, 3);

        cache.clear_all().unwrap();
        assert_eq!(cache.stats().unwrap().entry_count, 0);
    }

    #[test]
    fn test_stats() {
        let cache = make_cache();

        cache
            .put(
                "https://foo.com/a",
                "aaa",
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
        cache
            .put(
                "https://foo.com/b",
                "bbb",
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
        cache
            .put(
                "https://bar.com/c",
                "ccc",
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();

        let stats = cache.stats().unwrap();
        assert_eq!(stats.entry_count, 3);
        assert!(stats.total_size_bytes > 0);
        assert!(stats.top_domains.len() >= 2);
        let foo_count = stats
            .top_domains
            .iter()
            .find(|(d, _)| d == "foo.com")
            .map(|(_, c)| *c);
        assert_eq!(foo_count, Some(2));

        let bar_count = stats
            .top_domains
            .iter()
            .find(|(d, _)| d == "bar.com")
            .map(|(_, c)| *c);
        assert_eq!(bar_count, Some(1));
    }

    #[test]
    fn test_put_with_conditional() {
        let cache = make_cache();

        let cond = ConditionalHeaders {
            etag: Some("\"abc123\"".to_string()),
            last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".to_string()),
        };

        cache
            .put_with_conditional(
                "https://example.com/etag-test",
                "content with etag",
                "text/html",
                Duration::from_secs(3600),
                Some(&cond),
            )
            .unwrap();

        let headers = cache
            .get_conditional_headers("https://example.com/etag-test")
            .unwrap();
        assert_eq!(headers.etag, Some("\"abc123\"".to_string()));
        assert_eq!(
            headers.last_modified,
            Some("Wed, 21 Oct 2015 07:28:00 GMT".to_string())
        );
    }

    #[test]
    fn test_conditional_headers_missing_url() {
        let cache = make_cache();
        let headers = cache
            .get_conditional_headers("https://example.com/nonexistent")
            .unwrap();
        assert!(headers.etag.is_none());
        assert!(headers.last_modified.is_none());
    }

    #[test]
    fn test_content_hash_unchanged() {
        let cache = make_cache();
        let content = "same content";

        cache
            .put(
                "https://example.com/hash-test",
                content,
                "text/plain",
                Duration::from_secs(3600),
            )
            .unwrap();

        assert!(cache
            .content_hash_unchanged("https://example.com/hash-test", content)
            .unwrap());
        assert!(!cache
            .content_hash_unchanged("https://example.com/hash-test", "different content")
            .unwrap());
        assert!(!cache
            .content_hash_unchanged("https://example.com/unknown", content)
            .unwrap());
    }

    #[test]
    fn test_find_by_hash() {
        let cache = make_cache();
        let shared = "identical content";

        cache
            .put(
                "https://a.com/page",
                shared,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
        cache
            .put(
                "https://b.com/page",
                shared,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
        cache
            .put(
                "https://c.com/other",
                "different",
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();

        let hash = sha256_hex(shared.as_bytes());
        let urls = cache.find_by_hash(&hash).unwrap();
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn test_dedup_removes_duplicates() {
        let cache = make_cache();
        let shared = "duplicate content";

        cache
            .put(
                "https://a.com/1",
                shared,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
        cache
            .put(
                "https://b.com/2",
                shared,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
        cache
            .put(
                "https://c.com/3",
                shared,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();

        let removed = cache.dedup().unwrap();
        assert_eq!(removed, 2);

        let hash = sha256_hex(shared.as_bytes());
        let remaining = cache.find_by_hash(&hash).unwrap();
        assert_eq!(remaining.len(), 1);
    }
}
