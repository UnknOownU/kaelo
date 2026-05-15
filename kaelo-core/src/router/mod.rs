pub mod fallback;
pub mod probe;
pub mod seed_domains;

use std::sync::Arc;

use anyhow::Result;

use crate::storage::route_cache::{FetchResult, RouteCache};
use crate::storage::Storage;
use crate::types::Strategy;

// ---------------------------------------------------------------------------
// RouterDecision
// ---------------------------------------------------------------------------

/// The outcome of resolving a URL through the router.
#[derive(Debug, Clone, PartialEq)]
pub enum RouterDecision {
    /// Known domain — a cached strategy with proven performance.
    Known { strategy: Strategy, score: f64 },
    /// Unknown domain — fall back to the default strategy.
    Unknown { strategy: Strategy },
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Maps a URL to the best fetch strategy by consulting the route cache.
pub struct Router {
    storage: Arc<Storage>,
    default_strategy: Strategy,
}

impl Router {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            default_strategy: Strategy::HttpSimple,
        }
    }

    pub fn with_default_strategy(mut self, strategy: Strategy) -> Self {
        self.default_strategy = strategy;
        self
    }

    /// Resolve the best fetch strategy for `url`.
    ///
    /// 1. Extract domain from URL.
    /// 2. Look up route cache for known strategies.
    /// 3. Return best cached strategy or the default.
    pub fn resolve(&self, url: &str) -> Result<RouterDecision> {
        let domain = extract_domain(url)?;
        let cache = RouteCache::new(&self.storage);

        if let Some(cached) = cache.get_best_strategy(&domain)? {
            let strategy = parse_strategy(&cached.strategy)?;
            let s = compute_score(cached.success_rate, cached.avg_latency_ms);
            return Ok(RouterDecision::Known { strategy, score: s });
        }

        Ok(RouterDecision::Unknown {
            strategy: self.default_strategy.clone(),
        })
    }

    pub fn record_outcome(
        &self,
        url: &str,
        strategy: &Strategy,
        latency_ms: u64,
        success: bool,
    ) -> Result<()> {
        let domain = extract_domain(url)?;
        let cache = RouteCache::new(&self.storage);
        let result = FetchResult {
            latency_ms,
            success,
            headers: None,
        };
        let strategy_name = format!("{:?}", strategy);
        cache.upsert_strategy(&domain, &strategy_name, &result)?;
        Ok(())
    }

    pub fn ensure_seeded(&self) -> Result<bool> {
        let cache = RouteCache::new(&self.storage);
        if cache.count_entries()? == 0 {
            seed_domains::import_seeds(&cache)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the host from a URL using simple string parsing.
fn extract_domain(url: &str) -> Result<String> {
    let no_proto = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| anyhow::anyhow!("invalid URL: no protocol"))?;

    let domain = no_proto.split('/').next().unwrap_or(no_proto);
    let domain = domain.split(':').next().unwrap_or(domain);
    Ok(domain.to_string())
}

/// Map a strategy name string back to the `Strategy` enum.
fn parse_strategy(name: &str) -> Result<Strategy> {
    match name {
        "HttpSimple" => Ok(Strategy::HttpSimple),
        "TlsChrome" => Ok(Strategy::TlsChrome),
        "TlsMobile" => Ok(Strategy::TlsMobile),
        "Headless" => Ok(Strategy::Headless),
        "PublicApi" => Ok(Strategy::PublicApi),
        other => Err(anyhow::anyhow!("unknown strategy: {}", other)),
    }
}

/// score = success_rate * (1 / max(latency_s, 0.001))
fn compute_score(success_rate: f64, avg_latency_ms: u64) -> f64 {
    success_rate * (1.0 / (avg_latency_ms as f64 / 1000.0).max(0.001))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
mod tests {
    use super::*;
    use crate::storage::route_cache::FetchResult;

    /// Seed the route cache with a strategy for the given domain.
    fn seed_strategy(
        storage: &Storage,
        domain: &str,
        strategy: &str,
        latency_ms: u64,
        success: bool,
    ) {
        let cache = RouteCache::new(storage);
        cache
            .upsert_strategy(
                domain,
                strategy,
                &FetchResult {
                    latency_ms,
                    success,
                    headers: None,
                },
            )
            .expect("upsert should succeed");
    }

    // ---- resolve ----

    #[test]
    fn test_resolve_known_domain() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        seed_strategy(&storage, "reddit.com", "TlsMobile", 1500, true);
        // Second success to push rate to 1.0
        seed_strategy(&storage, "reddit.com", "TlsMobile", 1200, true);

        let router = Router::new(Arc::clone(&storage));
        let decision = router.resolve("https://reddit.com/r/rust").unwrap();

        match decision {
            RouterDecision::Known { strategy, score } => {
                assert_eq!(strategy, Strategy::TlsMobile);
                assert!(score > 0.0);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known decision"),
        }
    }

    #[test]
    fn test_resolve_unknown_domain() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        let router = Router::new(storage);
        let decision = router.resolve("https://example.com/page").unwrap();

        match decision {
            RouterDecision::Unknown { strategy } => {
                assert_eq!(strategy, Strategy::HttpSimple);
            }
            RouterDecision::Known { .. } => panic!("expected Unknown decision"),
        }
    }

    #[test]
    fn test_default_strategy_customizable() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        let router = Router::new(storage).with_default_strategy(Strategy::Headless);
        let decision = router.resolve("https://unknown.example/path").unwrap();

        match decision {
            RouterDecision::Unknown { strategy } => {
                assert_eq!(strategy, Strategy::Headless);
            }
            RouterDecision::Known { .. } => panic!("expected Unknown decision"),
        }
    }

    // ---- extract_domain ----

    #[test]
    fn test_extract_domain_various() {
        assert_eq!(
            extract_domain("https://example.com/path").unwrap(),
            "example.com"
        );
        assert_eq!(
            extract_domain("http://example.com:8080/path").unwrap(),
            "example.com"
        );
        assert_eq!(
            extract_domain("https://sub.domain.org/resource?q=1").unwrap(),
            "sub.domain.org"
        );
        assert_eq!(
            extract_domain("http://localhost:3000/api").unwrap(),
            "localhost"
        );
        assert!(extract_domain("ftp://example.com").is_err());
    }

    // ---- parse_strategy ----

    #[test]
    fn test_parse_strategy_variants() {
        assert_eq!(parse_strategy("HttpSimple").unwrap(), Strategy::HttpSimple);
        assert_eq!(parse_strategy("TlsChrome").unwrap(), Strategy::TlsChrome);
        assert_eq!(parse_strategy("TlsMobile").unwrap(), Strategy::TlsMobile);
        assert_eq!(parse_strategy("Headless").unwrap(), Strategy::Headless);
        assert_eq!(parse_strategy("PublicApi").unwrap(), Strategy::PublicApi);
        assert!(parse_strategy("Unknown").is_err());
    }

    // ---- record_outcome ----

    #[test]
    fn test_record_outcome_success() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        let router = Router::new(Arc::clone(&storage));

        seed_strategy(&storage, "reddit.com", "TlsMobile", 1500, true);

        router
            .record_outcome(
                "https://reddit.com/r/rust",
                &Strategy::TlsMobile,
                1200,
                true,
            )
            .unwrap();

        let decision = router.resolve("https://reddit.com/r/rust").unwrap();
        match decision {
            RouterDecision::Known { strategy, score } => {
                assert_eq!(strategy, Strategy::TlsMobile);
                assert!(score > 0.0);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known after recording outcome"),
        }
    }

    #[test]
    fn test_record_outcome_failure() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        let router = Router::new(Arc::clone(&storage));

        seed_strategy(&storage, "scrape.me", "HttpSimple", 500, true);
        router
            .record_outcome("https://scrape.me/page", &Strategy::HttpSimple, 8000, false)
            .unwrap();

        let cache = RouteCache::new(&storage);
        let best = cache.get_best_strategy("scrape.me").unwrap().unwrap();
        assert!(best.success_rate < 1.0);
        assert_eq!(best.total_requests, 2);
    }

    // ---- scoring ----

    #[test]
    fn test_scoring_higher_success_wins() {
        let high = compute_score(1.0, 1000);
        let low = compute_score(0.5, 1000);
        assert!(high > low);
    }

    #[test]
    fn test_scoring_lower_latency_wins() {
        let fast = compute_score(1.0, 100);
        let slow = compute_score(1.0, 2000);
        assert!(fast > slow);
    }

    // ---- ensure_seeded ----

    #[test]
    fn test_ensure_seeded_first_time() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        let router = Router::new(Arc::clone(&storage));

        let seeded = router
            .ensure_seeded()
            .expect("ensure_seeded should succeed");
        assert!(seeded, "empty cache should be seeded");

        let cache = RouteCache::new(&storage);
        assert_eq!(
            cache.count_entries().unwrap(),
            seed_domains::SEED_DOMAINS.len() as u64
        );
    }

    #[test]
    fn test_ensure_seeded_skip() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        let router = Router::new(Arc::clone(&storage));

        seed_strategy(&storage, "example.com", "HttpSimple", 500, true);

        let seeded = router
            .ensure_seeded()
            .expect("ensure_seeded should succeed");
        assert!(!seeded, "non-empty cache should not be re-seeded");

        let cache = RouteCache::new(&storage);
        assert_eq!(cache.count_entries().unwrap(), 1);
    }
}
