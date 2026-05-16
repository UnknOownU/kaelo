use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use super::Storage;

const DEFAULT_MAX_ENTRIES: usize = 10_000;

pub fn max_entries() -> usize {
    std::env::var("KAELO_MAX_ROUTE_ENTRIES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_ENTRIES)
}

#[derive(Debug, Clone)]
pub struct CachedStrategy {
    pub domain: String,
    pub strategy: String,
    pub avg_latency_ms: u64,
    pub success_rate: f64,
    pub total_requests: u32,
    pub last_success_at: Option<String>,
    pub last_check_at: String,
    pub extra_headers: Option<String>,
}

pub struct FetchResult {
    pub latency_ms: u64,
    pub success: bool,
    pub headers: Option<HashMap<String, String>>,
}

fn score(success_rate: f64, avg_latency_ms: u64) -> f64 {
    success_rate * (1.0 / (avg_latency_ms as f64 / 1000.0).max(0.001))
}

pub struct RouteCache<'a> {
    storage: &'a Storage,
}

impl<'a> RouteCache<'a> {
    pub fn new(storage: &'a Storage) -> Self {
        Self { storage }
    }

    pub fn get_strategies(&self, domain: &str) -> Result<Vec<CachedStrategy>> {
        let conn = self.storage.conn();
        let mut stmt = conn.prepare(
            "SELECT domain, strategy, avg_latency_ms, success_rate, total_requests,
                    last_success_at, last_check_at, extra_headers
             FROM domain_strategies
             WHERE domain = ?1",
        )?;

        let rows: Vec<CachedStrategy> = stmt
            .query_map([domain], |row| {
                Ok(CachedStrategy {
                    domain: row.get(0)?,
                    strategy: row.get(1)?,
                    avg_latency_ms: row.get(2)?,
                    success_rate: row.get(3)?,
                    total_requests: row.get(4)?,
                    last_success_at: row.get(5)?,
                    last_check_at: row.get(6)?,
                    extra_headers: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        let mut sorted = rows;
        sorted.sort_by(|a, b| {
            score(b.success_rate, b.avg_latency_ms)
                .partial_cmp(&score(a.success_rate, a.avg_latency_ms))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(sorted)
    }

    pub fn get_best_strategy(&self, domain: &str) -> Result<Option<CachedStrategy>> {
        let strategies = self.get_strategies(domain)?;
        Ok(strategies.into_iter().next())
    }

    /// Insert a new (domain, strategy) entry, or update the existing one with
    /// running-average latency / success-rate and an incremented request count.
    pub fn upsert_strategy(
        &self,
        domain: &str,
        strategy: &str,
        result: &FetchResult,
    ) -> Result<()> {
        let conn = self.storage.conn();
        let now = now_iso();

        let existing: Option<(u64, f64, u32)> = conn
            .prepare(
                "SELECT avg_latency_ms, success_rate, total_requests
                 FROM domain_strategies
                 WHERE domain = ?1 AND strategy = ?2",
            )?
            .query_row([domain, strategy], |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, u32>(2)?,
                ))
            })
            .ok();

        let headers_json = result
            .headers
            .as_ref()
            .map(|h| serde_json::to_string(h).unwrap_or_default());

        match existing {
            Some((prev_latency, prev_rate, prev_count)) => {
                let new_count = prev_count + 1;
                let new_latency = running_avg(prev_latency, prev_count, result.latency_ms);
                let success_val = if result.success { 1.0_f64 } else { 0.0 };
                let new_rate = running_avg_f64(prev_rate, prev_count, success_val);

                let success_at = if result.success {
                    Some(now.clone())
                } else {
                    conn.query_row(
                        "SELECT last_success_at FROM domain_strategies
                         WHERE domain = ?1 AND strategy = ?2",
                        [domain, strategy],
                        |row| row.get(0),
                    )
                    .ok()
                    .flatten()
                };

                conn.execute(
                    "UPDATE domain_strategies
                     SET avg_latency_ms = ?1, success_rate = ?2, total_requests = ?3,
                         last_success_at = ?4, last_check_at = ?5, extra_headers = ?6
                     WHERE domain = ?7 AND strategy = ?8",
                    (
                        new_latency,
                        new_rate,
                        new_count,
                        success_at,
                        &now,
                        &headers_json,
                        domain,
                        strategy,
                    ),
                )?;
            }
            None => {
                let initial_rate = if result.success { 1.0 } else { 0.0 };
                let success_at = if result.success {
                    Some(now.clone())
                } else {
                    None
                };

                conn.execute(
                    "INSERT INTO domain_strategies
                        (domain, strategy, avg_latency_ms, success_rate, total_requests,
                         last_success_at, last_check_at, extra_headers)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    (
                        domain,
                        strategy,
                        result.latency_ms,
                        initial_rate,
                        1u32,
                        success_at,
                        &now,
                        &headers_json,
                    ),
                )?;
            }
        }

        let count: u64 = conn.query_row("SELECT COUNT(*) FROM domain_strategies", [], |row| {
            row.get(0)
        })?;
        let max = max_entries();
        if count > max as u64 {
            self.evict_oldest(count as usize - max)?;
        }

        Ok(())
    }

    pub fn remove_strategy(&self, domain: &str, strategy: &str) -> Result<()> {
        let conn = self.storage.conn();
        conn.execute(
            "DELETE FROM domain_strategies WHERE domain = ?1 AND strategy = ?2",
            [domain, strategy],
        )?;
        Ok(())
    }

    pub fn evict_stale(
        &self,
        max_age_days: u32,
        min_success_rate: f64,
        min_requests: u32,
    ) -> Result<u64> {
        let conn = self.storage.conn();
        let cutoff = now_iso_minus_days(max_age_days);

        let deleted = conn.execute(
            "DELETE FROM domain_strategies
             WHERE total_requests >= ?1
               AND success_rate < ?2
               AND last_check_at < ?3",
            rusqlite::params![min_requests, min_success_rate, cutoff],
        )?;

        Ok(deleted as u64)
    }

    pub fn evict_oldest(&self, limit: usize) -> Result<u64> {
        if limit == 0 {
            return Ok(0);
        }
        let conn = self.storage.conn();
        let deleted = conn.execute(
            "DELETE FROM domain_strategies WHERE rowid IN (
                SELECT rowid FROM domain_strategies ORDER BY last_check_at ASC LIMIT ?1
            )",
            rusqlite::params![limit as i64],
        )?;
        Ok(deleted as u64)
    }

    pub fn get_all_domains(&self) -> Result<Vec<String>> {
        let conn = self.storage.conn();
        let mut stmt = conn.prepare("SELECT DISTINCT domain FROM domain_strategies")?;
        let domains: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(domains)
    }

    pub fn count_entries(&self) -> Result<u64> {
        let conn = self.storage.conn();
        let count: u64 = conn.query_row("SELECT COUNT(*) FROM domain_strategies", [], |row| {
            row.get(0)
        })?;
        Ok(count)
    }

    pub fn get_all_strategies(&self) -> Result<Vec<CachedStrategy>> {
        let conn = self.storage.conn();
        let mut stmt = conn.prepare(
            "SELECT domain, strategy, avg_latency_ms, success_rate, total_requests,
                    last_success_at, last_check_at, extra_headers
             FROM domain_strategies",
        )?;

        let rows: Vec<CachedStrategy> = stmt
            .query_map([], |row| {
                Ok(CachedStrategy {
                    domain: row.get(0)?,
                    strategy: row.get(1)?,
                    avg_latency_ms: row.get(2)?,
                    success_rate: row.get(3)?,
                    total_requests: row.get(4)?,
                    last_success_at: row.get(5)?,
                    last_check_at: row.get(6)?,
                    extra_headers: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn export_to_json(&self) -> Result<String> {
        let strategies = self.get_all_strategies()?;

        #[derive(serde::Serialize)]
        struct ExportEntry {
            domain: String,
            strategy: String,
            avg_latency_ms: u64,
            success_rate: f64,
            total_requests: u32,
        }

        let entries: Vec<ExportEntry> = strategies
            .into_iter()
            .map(|s| ExportEntry {
                domain: s.domain,
                strategy: s.strategy,
                avg_latency_ms: s.avg_latency_ms,
                success_rate: s.success_rate,
                total_requests: s.total_requests,
            })
            .collect();

        let json =
            serde_json::to_string_pretty(&entries).context("failed to serialize route cache")?;
        Ok(json)
    }

    /// Deserialize entries from a JSON string and upsert them (merge with existing).
    pub fn import_from_json(&self, json: &str) -> Result<usize> {
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ImportEntry {
            domain: String,
            strategy: String,
            avg_latency_ms: u64,
            success_rate: f64,
            total_requests: u32,
        }

        let entries: Vec<ImportEntry> =
            serde_json::from_str(json).context("failed to parse import JSON")?;

        let mut count = 0usize;
        for entry in &entries {
            // Use upsert_strategy with a synthetic FetchResult so it merges
            // running averages with any existing entry instead of overwriting.
            let result = FetchResult {
                latency_ms: entry.avg_latency_ms,
                success: entry.success_rate > 0.5,
                headers: None,
            };
            self.upsert_strategy(&entry.domain, &entry.strategy, &result)?;
            count += 1;
        }

        Ok(count)
    }

    pub fn export_json(&self, path: &Path) -> Result<()> {
        let strategies = self.get_all_strategies()?;

        #[derive(serde::Serialize)]
        struct ExportEntry {
            domain: String,
            strategy: String,
            avg_latency_ms: u64,
            success_rate: f64,
            total_requests: u32,
        }

        let entries: Vec<ExportEntry> = strategies
            .into_iter()
            .map(|s| ExportEntry {
                domain: s.domain,
                strategy: s.strategy,
                avg_latency_ms: s.avg_latency_ms,
                success_rate: s.success_rate,
                total_requests: s.total_requests,
            })
            .collect();

        let json =
            serde_json::to_string_pretty(&entries).context("failed to serialize route cache")?;

        std::fs::write(path, json).context("failed to write export file")?;

        Ok(())
    }

    pub fn import_json(&self, path: &Path) -> Result<u64> {
        let content = std::fs::read_to_string(path).context("failed to read import file")?;

        #[derive(serde::Deserialize)]
        struct ImportEntry {
            domain: String,
            strategy: String,
            avg_latency_ms: u64,
            success_rate: f64,
            total_requests: u32,
        }

        let entries: Vec<ImportEntry> =
            serde_json::from_str(&content).context("failed to parse import JSON")?;

        let mut count: u64 = 0;
        for entry in &entries {
            let conn = self.storage.conn();
            conn.execute(
                "INSERT OR REPLACE INTO domain_strategies
                    (domain, strategy, avg_latency_ms, success_rate, total_requests,
                     last_success_at, last_check_at, extra_headers)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                rusqlite::params![
                    entry.domain,
                    entry.strategy,
                    entry.avg_latency_ms,
                    entry.success_rate,
                    entry.total_requests,
                    now_iso(),
                    now_iso(),
                ],
            )?;

            count += 1;
        }

        Ok(count)
    }

    pub fn import_pack(&self, path: &Path) -> Result<u64> {
        let content = std::fs::read_to_string(path).context("failed to read pack file")?;

        #[derive(serde::Deserialize)]
        struct PackDomain {
            domain: String,
            strategy: String,
        }

        #[derive(serde::Deserialize)]
        struct RoutePack {
            #[allow(dead_code)]
            name: String,
            #[allow(dead_code)]
            version: String,
            domains: Vec<PackDomain>,
        }

        let pack: RoutePack =
            serde_json::from_str(&content).context("failed to parse route pack JSON")?;

        let mut count: u64 = 0;
        for entry in &pack.domains {
            let result = FetchResult {
                latency_ms: 0,
                success: true,
                headers: None,
            };
            self.upsert_strategy(&entry.domain, &entry.strategy, &result)?;
            count += 1;
        }

        Ok(count)
    }
}

fn running_avg(prev_avg: u64, prev_count: u32, new_val: u64) -> u64 {
    let n = prev_count as u64;
    ((prev_avg * n) + new_val) / (n + 1)
}

fn running_avg_f64(prev_avg: f64, prev_count: u32, new_val: f64) -> f64 {
    let n = prev_count as f64;
    (prev_avg * n + new_val) / (n + 1.0)
}

fn now_iso() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    chrono_less_iso(duration.as_secs() as i64)
}

fn now_iso_minus_days(days: u32) -> String {
    let secs_in_day: u64 = 86_400;
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let past = duration.as_secs().saturating_sub(secs_in_day * days as u64);
    chrono_less_iso(past as i64)
}

fn chrono_less_iso(unix_secs: i64) -> String {
    const CUM_DAYS: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

    let mut secs = unix_secs;
    let mut days = secs / 86_400;
    secs %= 86_400;
    if secs < 0 {
        secs += 86_400;
        days -= 1;
    }

    let mut year = 1970;
    let mut remaining = days;

    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        year += 1;
    }

    let leap = is_leap(year);
    let mut month = 0usize;
    for (m, &cum_base) in CUM_DAYS.iter().enumerate().skip(1) {
        let mut cum = cum_base;
        if leap && m > 1 {
            cum += 1;
        }
        if remaining < cum {
            break;
        }
        month = m;
    }

    let mut day_offset = CUM_DAYS[month];
    if leap && month > 1 {
        day_offset += 1;
    }
    let day = remaining - day_offset + 1;

    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month + 1,
        day + 1,
        h,
        m,
        s
    )
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_storage() -> Storage {
        Storage::open(":memory:").expect("in-memory storage should open")
    }

    fn ok_result(latency_ms: u64) -> FetchResult {
        FetchResult {
            latency_ms,
            success: true,
            headers: None,
        }
    }

    fn fail_result(latency_ms: u64) -> FetchResult {
        FetchResult {
            latency_ms,
            success: false,
            headers: None,
        }
    }

    #[test]
    fn test_insert_and_get() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("example.com", "TlsMobile", &ok_result(200))
            .unwrap();
        cache
            .upsert_strategy("example.com", "Headless", &ok_result(300))
            .unwrap();

        let strategies = cache.get_strategies("example.com").unwrap();
        assert_eq!(strategies.len(), 3);

        let names: Vec<&str> = strategies.iter().map(|s| s.strategy.as_str()).collect();
        assert!(names.contains(&"HttpSimple"));
        assert!(names.contains(&"TlsMobile"));
        assert!(names.contains(&"Headless"));
    }

    #[test]
    fn test_best_strategy_highest_score() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        // Both succeed; lower latency → higher score.
        cache
            .upsert_strategy("example.com", "SlowStrategy", &ok_result(2000))
            .unwrap();
        cache
            .upsert_strategy("example.com", "FastStrategy", &ok_result(50))
            .unwrap();

        let best = cache.get_best_strategy("example.com").unwrap().unwrap();
        assert_eq!(best.strategy, "FastStrategy");
        assert!(best.avg_latency_ms < 2000);
    }

    #[test]
    fn test_upsert_updates_existing() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(100))
            .unwrap();

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(200))
            .unwrap();

        let strategies = cache.get_strategies("example.com").unwrap();
        assert_eq!(strategies.len(), 1);

        let s = &strategies[0];
        assert_eq!(s.total_requests, 2);
        // Running avg of 100 and 200 = 150.
        assert_eq!(s.avg_latency_ms, 150);
        assert!(s.success_rate > 0.99); // 2/2 successes.
    }

    #[test]
    fn test_composite_pk() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("example.com", "TlsMobile", &ok_result(200))
            .unwrap();

        let strategies = cache.get_strategies("example.com").unwrap();
        assert_eq!(strategies.len(), 2);

        cache.remove_strategy("example.com", "HttpSimple").unwrap();
        let strategies = cache.get_strategies("example.com").unwrap();
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].strategy, "TlsMobile");
    }

    #[test]
    fn test_eviction_removes_stale() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        let conn = storage.conn();
        conn.execute(
            "INSERT INTO domain_strategies
                (domain, strategy, avg_latency_ms, success_rate, total_requests,
                 last_success_at, last_check_at, extra_headers)
             VALUES ('old.com', 'DeadStrategy', 5000, 0.2, 10, NULL, '2020-01-01T00:00:00Z', NULL)",
            [],
        )
        .unwrap();

        cache
            .upsert_strategy("old.com", "GoodStrategy", &ok_result(100))
            .unwrap();

        let removed = cache.evict_stale(30, 0.5, 5).unwrap();
        assert_eq!(removed, 1);

        let remaining = cache.get_strategies("old.com").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].strategy, "GoodStrategy");
    }

    #[test]
    fn test_get_all_domains() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("a.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("b.com", "HttpSimple", &ok_result(200))
            .unwrap();
        cache
            .upsert_strategy("c.com", "HttpSimple", &ok_result(300))
            .unwrap();

        let mut domains = cache.get_all_domains().unwrap();
        domains.sort();
        assert_eq!(domains, vec!["a.com", "b.com", "c.com"]);
    }

    #[test]
    fn test_score_calculation() {
        let s1 = score(1.0, 1000);
        assert!((s1 - 1.0).abs() < f64::EPSILON);

        let s2 = score(0.5, 500);
        assert!((s2 - 1.0).abs() < f64::EPSILON);

        let s3 = score(0.0, 1000);
        assert!(s3.abs() < f64::EPSILON);
    }

    #[test]
    fn test_count_entries() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        assert_eq!(cache.count_entries().unwrap(), 0);

        cache
            .upsert_strategy("a.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("b.com", "TlsMobile", &ok_result(200))
            .unwrap();

        assert_eq!(cache.count_entries().unwrap(), 2);
    }

    #[test]
    fn test_upsert_failure_lowers_rate() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("flaky.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("flaky.com", "HttpSimple", &fail_result(500))
            .unwrap();

        let s = &cache.get_strategies("flaky.com").unwrap()[0];
        assert_eq!(s.total_requests, 2);
        assert!((s.success_rate - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_export_import_json() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("a.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("b.com", "TlsMobile", &ok_result(200))
            .unwrap();

        let dir = std::env::temp_dir().join("kaelo_test_export");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("routes.json");

        cache.export_json(&path).unwrap();
        assert!(path.exists());

        let storage2 = make_storage();
        let cache2 = RouteCache::new(&storage2);
        let count = cache2.import_json(&path).unwrap();
        assert_eq!(count, 2);

        let imported = cache2.get_all_strategies().unwrap();
        assert_eq!(imported.len(), 2);
    }

    #[test]
    fn test_import_pack() {
        let dir = std::env::temp_dir().join("kaelo_test_pack");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pack.json");

        let pack_json = r#"{"name":"test-pack","version":"1.0","domains":[{"domain":"example.com","strategy":"TlsMobile"},{"domain":"github.com","strategy":"HttpSimple"}]}"#;
        std::fs::write(&path, pack_json).unwrap();

        let storage = make_storage();
        let cache = RouteCache::new(&storage);
        let count = cache.import_pack(&path).unwrap();
        assert_eq!(count, 2);

        let best = cache.get_best_strategy("example.com").unwrap().unwrap();
        assert_eq!(best.strategy, "TlsMobile");
    }

    #[test]
    fn test_export_to_json_empty_cache() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        let json = cache.export_to_json().unwrap();
        assert_eq!(json.trim(), "[]");
    }

    #[test]
    fn test_export_import_roundtrip() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(100))
            .unwrap();
        cache
            .upsert_strategy("example.com", "TlsMobile", &ok_result(200))
            .unwrap();
        cache
            .upsert_strategy("github.com", "Headless", &fail_result(500))
            .unwrap();

        let json = cache.export_to_json().unwrap();
        assert!(json.contains("example.com"));
        assert!(json.contains("github.com"));

        let storage2 = make_storage();
        let cache2 = RouteCache::new(&storage2);
        let count = cache2.import_from_json(&json).unwrap();
        assert_eq!(count, 3);

        assert!(cache2.get_best_strategy("example.com").unwrap().is_some());
        assert!(cache2.get_best_strategy("github.com").unwrap().is_some());
    }

    #[test]
    fn test_import_from_json_invalid() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        let result = cache.import_from_json("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_import_from_json_merges_existing() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(100))
            .unwrap();
        let original = cache.get_best_strategy("example.com").unwrap().unwrap();
        assert_eq!(original.total_requests, 1);

        let json = r#"[
            {"domain":"example.com","strategy":"HttpSimple","avg_latency_ms":200,"success_rate":1.0,"total_requests":1}
        ]"#;
        let count = cache.import_from_json(json).unwrap();
        assert_eq!(count, 1);

        let merged = cache.get_best_strategy("example.com").unwrap().unwrap();
        assert_eq!(merged.total_requests, 2);
    }

    #[test]
    fn test_lru_eviction_at_limit() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        for i in 0..11 {
            let domain = format!("site{}.com", i);
            cache
                .upsert_strategy(&domain, "HttpSimple", &ok_result(100))
                .unwrap();
        }

        assert_eq!(cache.count_entries().unwrap(), 11);

        let evicted = cache.evict_oldest(1).unwrap();
        assert_eq!(evicted, 1);
        assert_eq!(cache.count_entries().unwrap(), 10);
    }

    #[test]
    fn test_lru_eviction_oldest_removed() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        for i in 0..3 {
            let domain = format!("old{}.com", i);
            cache
                .upsert_strategy(&domain, "HttpSimple", &ok_result(100))
                .unwrap();
        }

        let conn = storage.conn();
        conn.execute(
            "UPDATE domain_strategies SET last_check_at = '2020-01-01T00:00:00Z'
             WHERE domain LIKE 'old%'",
            [],
        )
        .unwrap();

        for i in 0..3 {
            let domain = format!("new{}.com", i);
            cache
                .upsert_strategy(&domain, "HttpSimple", &ok_result(100))
                .unwrap();
        }

        cache.evict_oldest(3).unwrap();

        assert_eq!(cache.count_entries().unwrap(), 3);

        let domains = cache.get_all_domains().unwrap();
        for d in &domains {
            assert!(
                !d.starts_with("old"),
                "oldest entry should have been evicted, found {d}"
            );
        }
    }

    #[test]
    fn test_lru_newest_preserved() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        for i in 0..4 {
            let domain = format!("site{}.com", i);
            cache
                .upsert_strategy(&domain, "HttpSimple", &ok_result(100))
                .unwrap();
        }

        cache.evict_oldest(1).unwrap();

        assert_eq!(cache.count_entries().unwrap(), 3);

        let best = cache.get_best_strategy("site3.com").unwrap();
        assert!(best.is_some(), "newest entry should be preserved");

        let best = cache.get_best_strategy("site0.com").unwrap();
        assert!(best.is_none(), "oldest entry should be evicted");
    }

    #[test]
    fn test_max_entries_env_var_override() {
        std::env::set_var("KAELO_MAX_ROUTE_ENTRIES", "42");
        assert_eq!(max_entries(), 42);
        std::env::remove_var("KAELO_MAX_ROUTE_ENTRIES");
        assert_eq!(max_entries(), DEFAULT_MAX_ENTRIES);
    }

    #[test]
    fn test_evict_oldest_zero_limit() {
        let storage = make_storage();
        let cache = RouteCache::new(&storage);

        cache
            .upsert_strategy("example.com", "HttpSimple", &ok_result(100))
            .unwrap();

        let removed = cache.evict_oldest(0).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(cache.count_entries().unwrap(), 1);
    }
}
