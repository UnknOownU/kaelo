pub mod fallback;
pub mod probe;
pub mod seed_domains;

use std::sync::Arc;

use anyhow::Result;

use crate::router::probe::StrategyProber;
use crate::storage::route_cache::{FetchResult, RouteCache};
use crate::storage::Storage;
use crate::types::Strategy;

#[derive(Debug, Clone, PartialEq)]
pub enum RouterDecision {
    /// Known domain — a cached strategy with proven performance.
    Known { strategy: Strategy, score: f64 },
    /// Unknown domain — fall back to the default strategy.
    Unknown { strategy: Strategy },
}

pub struct Router {
    storage: Arc<Storage>,
    default_strategy: Strategy,
    prober: Option<Arc<dyn StrategyProber>>,
}

impl Router {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            default_strategy: Strategy::HttpSimple,
            prober: None,
        }
    }

    pub fn with_default_strategy(mut self, strategy: Strategy) -> Self {
        self.default_strategy = strategy;
        self
    }

    pub fn with_prober(mut self, prober: Arc<dyn StrategyProber>) -> Self {
        self.prober = Some(prober);
        self
    }

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

    /// Resolve the best strategy, probing if the domain is unknown.
    ///
    /// - If `explicit_strategy` is `Some`, return it directly (no probing).
    /// - If the domain has a cached strategy, return it (no probing).
    /// - Otherwise, probe using multiple strategies and cache the winner.
    pub async fn resolve_with_probing(
        &self,
        url: &str,
        explicit_strategy: Option<&Strategy>,
    ) -> Result<RouterDecision> {
        if let Some(strategy) = explicit_strategy {
            return Ok(RouterDecision::Known {
                strategy: strategy.clone(),
                score: 1.0,
            });
        }

        let domain = extract_domain(url)?;
        let cache = RouteCache::new(&self.storage);

        if let Some(cached) = cache.get_best_strategy(&domain)? {
            let strategy = parse_strategy(&cached.strategy)?;
            let s = compute_score(cached.success_rate, cached.avg_latency_ms);
            return Ok(RouterDecision::Known { strategy, score: s });
        }

        if let Some(prober) = &self.prober {
            if let Some(winner) = probe::probe_and_cache(
                prober.as_ref(),
                &self.storage,
                &domain,
                &self.default_strategy,
            )
            .await
            {
                return Ok(RouterDecision::Known {
                    strategy: winner,
                    score: 0.5,
                });
            }
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

fn extract_domain(url: &str) -> Result<String> {
    let no_proto = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| anyhow::anyhow!("invalid URL: no protocol"))?;

    let domain = no_proto.split('/').next().unwrap_or(no_proto);
    let domain = domain.split(':').next().unwrap_or(domain);
    Ok(domain.to_string())
}

fn parse_strategy(name: &str) -> Result<Strategy> {
    match name {
        "HttpSimple" => Ok(Strategy::HttpSimple),
        "TlsChrome" => Ok(Strategy::TlsChrome),
        "TlsMobile" => Ok(Strategy::TlsMobile),
        "Headless" => Ok(Strategy::Headless),
        "PublicApi" => Ok(Strategy::PublicApi),
        "YouTube" => Ok(Strategy::YouTube),
        other => Err(anyhow::anyhow!("unknown strategy: {}", other)),
    }
}

/// score = success_rate * (1 / max(latency_s, 0.001))
fn compute_score(success_rate: f64, avg_latency_ms: u64) -> f64 {
    success_rate * (1.0 / (avg_latency_ms as f64 / 1000.0).max(0.001))
}

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
mod tests {
    use super::*;
    use crate::router::probe::StrategyProber;
    use crate::storage::route_cache::FetchResult;
    use crate::types::{FetchError, FetchResponse};
    use std::collections::HashMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Duration;

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

    #[test]
    fn test_resolve_known_domain() {
        let storage = Arc::new(Storage::open(":memory:").unwrap());
        seed_strategy(&storage, "reddit.com", "TlsMobile", 1500, true);
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

    #[test]
    fn test_parse_strategy_variants() {
        assert_eq!(parse_strategy("HttpSimple").unwrap(), Strategy::HttpSimple);
        assert_eq!(parse_strategy("TlsChrome").unwrap(), Strategy::TlsChrome);
        assert_eq!(parse_strategy("TlsMobile").unwrap(), Strategy::TlsMobile);
        assert_eq!(parse_strategy("Headless").unwrap(), Strategy::Headless);
        assert_eq!(parse_strategy("PublicApi").unwrap(), Strategy::PublicApi);
        assert!(parse_strategy("Unknown").is_err());
    }

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

    struct MockStrategyProber {
        responses: HashMap<String, Result<FetchResponse, FetchError>>,
    }

    impl MockStrategyProber {
        fn new() -> Self {
            Self {
                responses: HashMap::new(),
            }
        }

        fn respond_to(mut self, strategy: &str, result: Result<FetchResponse, FetchError>) -> Self {
            self.responses.insert(strategy.to_string(), result);
            self
        }
    }

    impl StrategyProber for MockStrategyProber {
        fn probe_with_strategy(
            &self,
            _url: &str,
            strategy: &Strategy,
            _timeout: Duration,
        ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, FetchError>> + Send + '_>> {
            let key = format!("{:?}", strategy);
            let result = self.responses.get(&key).cloned();
            Box::pin(std::future::ready(
                result.unwrap_or(Err(FetchError::NetworkError("no mock".to_string()))),
            ))
        }
    }

    fn ok_response() -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status: 200,
            headers: HashMap::new(),
            body: b"ok".to_vec(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(100),
        })
    }

    #[tokio::test]
    async fn test_resolve_with_probing_explicit_skips_probing() {
        let storage = Arc::new(Storage::open(":memory:").expect("storage"));
        let prober = Arc::new(MockStrategyProber::new());
        let router = Router::new(storage).with_prober(prober);

        let decision = router
            .resolve_with_probing("https://newsite.com/page", Some(&Strategy::Headless))
            .await
            .expect("resolve_with_probing");

        match decision {
            RouterDecision::Known { strategy, .. } => {
                assert_eq!(strategy, Strategy::Headless);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known for explicit strategy"),
        }
    }

    #[tokio::test]
    async fn test_resolve_with_probing_uses_cached() {
        let storage = Arc::new(Storage::open(":memory:").expect("storage"));
        seed_strategy(&storage, "cached.com", "TlsChrome", 300, true);

        let prober = Arc::new(MockStrategyProber::new());
        let router = Router::new(Arc::clone(&storage)).with_prober(prober);

        let decision = router
            .resolve_with_probing("https://cached.com/page", None)
            .await
            .expect("resolve_with_probing");

        match decision {
            RouterDecision::Known { strategy, score } => {
                assert_eq!(strategy, Strategy::TlsChrome);
                assert!(score > 0.0);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known for cached domain"),
        }
    }

    #[tokio::test]
    async fn test_resolve_with_probing_probes_unknown() {
        let storage = Arc::new(Storage::open(":memory:").expect("storage"));
        let prober = Arc::new(
            MockStrategyProber::new()
                .respond_to("HttpSimple", Err(FetchError::Timeout))
                .respond_to("TlsChrome", ok_response())
                .respond_to("TlsMobile", ok_response())
                .respond_to("Headless", ok_response()),
        );

        let router = Router::new(Arc::clone(&storage)).with_prober(prober);

        let decision = router
            .resolve_with_probing("https://unknown-site.com/page", None)
            .await
            .expect("resolve_with_probing");

        match decision {
            RouterDecision::Known { strategy, .. } => {
                assert_eq!(strategy, Strategy::TlsChrome);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known after probing"),
        }

        let cache = RouteCache::new(&storage);
        let cached = cache.get_best_strategy("unknown-site.com").expect("cache");
        assert!(cached.is_some());
        assert_eq!(cached.expect("cached").strategy, "TlsChrome");
    }

    #[tokio::test]
    async fn test_resolve_with_probing_all_fail_returns_default() {
        let storage = Arc::new(Storage::open(":memory:").expect("storage"));
        let prober = Arc::new(MockStrategyProber::new());

        let router = Router::new(Arc::clone(&storage)).with_prober(prober);

        let decision = router
            .resolve_with_probing("https://dead-site.com/page", None)
            .await
            .expect("resolve_with_probing");

        match decision {
            RouterDecision::Known { strategy, .. } => {
                assert_eq!(strategy, Strategy::HttpSimple);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known (default cached)"),
        }

        let cache = RouteCache::new(&storage);
        let cached = cache.get_best_strategy("dead-site.com").expect("cache");
        assert!(cached.is_some());
    }

    #[tokio::test]
    async fn test_resolve_with_probing_no_prober_returns_unknown() {
        let storage = Arc::new(Storage::open(":memory:").expect("storage"));
        let router = Router::new(storage);

        let decision = router
            .resolve_with_probing("https://no-prober.com/page", None)
            .await
            .expect("resolve_with_probing");

        match decision {
            RouterDecision::Unknown { strategy } => {
                assert_eq!(strategy, Strategy::HttpSimple);
            }
            RouterDecision::Known { .. } => panic!("expected Unknown without prober"),
        }
    }

    #[tokio::test]
    async fn test_resolve_with_probing_second_call_uses_cache() {
        let storage = Arc::new(Storage::open(":memory:").expect("storage"));
        let prober = Arc::new(
            MockStrategyProber::new()
                .respond_to("HttpSimple", ok_response())
                .respond_to("TlsChrome", ok_response())
                .respond_to("TlsMobile", ok_response())
                .respond_to("Headless", ok_response()),
        );

        let router =
            Router::new(Arc::clone(&storage)).with_prober(prober as Arc<dyn StrategyProber>);

        let first = router
            .resolve_with_probing("https://fresh.com/page", None)
            .await
            .expect("first resolve");
        assert!(matches!(first, RouterDecision::Known { .. }));

        let cache = RouteCache::new(&storage);
        let cached = cache.get_best_strategy("fresh.com").expect("cache");
        assert!(cached.is_some());

        let second = router
            .resolve_with_probing("https://fresh.com/other", None)
            .await
            .expect("second resolve");
        match second {
            RouterDecision::Known { score, .. } => {
                assert!(score > 0.0);
            }
            RouterDecision::Unknown { .. } => panic!("expected Known on second call"),
        }
    }
}
