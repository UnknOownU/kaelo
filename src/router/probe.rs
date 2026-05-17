use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::storage::route_cache::{FetchResult, RouteCache};
use crate::storage::Storage;
use crate::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use super::fallback::fallback_order;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    Accessible,
    Blocked { status: u16 },
    Redirect { location: String, status: u16 },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyProbeResult {
    /// The strategy that won the probe (lowest latency among successful).
    pub strategy: Strategy,
    /// Measured latency in milliseconds.
    pub latency_ms: u64,
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Defined in `types` so the router can use it without depending on
/// `fetch`. Concrete backends in `fetch` implement this trait.
pub trait Prober: Send + Sync {
    fn probe(
        &self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, FetchError>> + Send + '_>>;
}

/// Each strategy variant gets its own method so the concrete implementation
/// (in `fetch`) can dispatch to the right backend.
pub trait StrategyProber: Send + Sync {
    fn probe_with_strategy(
        &self,
        url: &str,
        strategy: &Strategy,
        timeout: Duration,
    ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, FetchError>> + Send + '_>>;
}

pub async fn probe_domain(prober: &dyn Prober, url: &str) -> ProbeResult {
    let mut headers = HashMap::new();
    headers.insert("Range".to_string(), "bytes=0-0".to_string());

    let request = FetchRequest {
        url: url.to_string(),
        headers,
        timeout: Duration::from_secs(10),
        follow_redirects: false,
    };

    match prober.probe(request).await {
        Ok(response) => classify_response(response),
        Err(e) => classify_error(e),
    }
}

/// Try multiple strategies in priority order and return the fastest successful
/// one. Each strategy gets a 5-second timeout. The total probe is bounded by
/// `strategies.len() * 5s`.
pub async fn probe_with_strategies(
    prober: &dyn StrategyProber,
    domain: &str,
    strategies: &[Strategy],
) -> Option<StrategyProbeResult> {
    let url = format!("https://{domain}/");
    let mut best: Option<StrategyProbeResult> = None;

    for strategy in strategies {
        let start = Instant::now();
        let result = prober
            .probe_with_strategy(&url, strategy, PROBE_TIMEOUT)
            .await;

        let latency = start.elapsed();
        let latency_ms = latency.as_millis() as u64;

        match result {
            Ok(response) if is_probe_success(&response) => {
                let candidate = StrategyProbeResult {
                    strategy: strategy.clone(),
                    latency_ms,
                };
                if best
                    .as_ref()
                    .map_or(true, |b| candidate.latency_ms < b.latency_ms)
                {
                    best = Some(candidate);
                }
            }
            _ => {}
        }
    }

    best
}

fn is_probe_success(response: &FetchResponse) -> bool {
    !matches!(response.status, 403 | 503)
}

/// Probe an unknown domain using multiple strategies, cache the winner, and
/// return it. Returns `None` if all strategies fail.
pub async fn probe_and_cache(
    prober: &dyn StrategyProber,
    storage: &Arc<Storage>,
    domain: &str,
    default_strategy: &Strategy,
) -> Option<Strategy> {
    let strategies: Vec<Strategy> = fallback_order()
        .into_iter()
        .filter(|s| !matches!(s, Strategy::PublicApi | Strategy::YouTube))
        .collect();

    let result = probe_with_strategies(prober, domain, &strategies).await;

    match result {
        Some(probe_result) => {
            let cache = RouteCache::new(storage);
            let fetch_result = FetchResult {
                latency_ms: probe_result.latency_ms,
                success: true,
                headers: None,
            };
            let strategy_name = format!("{:?}", probe_result.strategy);
            let _ = cache.upsert_strategy(domain, &strategy_name, &fetch_result);
            Some(probe_result.strategy)
        }
        None => {
            let cache = RouteCache::new(storage);
            let fetch_result = FetchResult {
                latency_ms: 5000,
                success: false,
                headers: None,
            };
            let strategy_name = format!("{:?}", default_strategy);
            let _ = cache.upsert_strategy(domain, &strategy_name, &fetch_result);
            Some(default_strategy.clone())
        }
    }
}

fn classify_response(response: FetchResponse) -> ProbeResult {
    match response.status {
        200 | 206 => ProbeResult::Accessible,

        301 | 302 | 307 | 308 => {
            let location = response
                .headers
                .get("location")
                .cloned()
                .unwrap_or_default();
            ProbeResult::Redirect {
                location,
                status: response.status,
            }
        }

        403 | 503 => ProbeResult::Blocked {
            status: response.status,
        },

        // 404 etc. — server responded, domain is reachable
        _ => ProbeResult::Accessible,
    }
}

fn classify_error(e: FetchError) -> ProbeResult {
    match e {
        FetchError::Timeout => ProbeResult::Error {
            message: "probe timed out".to_string(),
        },
        FetchError::HttpError(status) => {
            if status == 403 || status == 503 {
                ProbeResult::Blocked { status }
            } else {
                ProbeResult::Accessible
            }
        }
        FetchError::TlsError(msg)
        | FetchError::BrowserError(msg)
        | FetchError::NetworkError(msg) => ProbeResult::Error { message: msg },
    }
}

#[cfg(test)]
#[allow(clippy::arc_with_non_send_sync)]
mod tests {
    use super::*;

    fn response(status: u16, headers: HashMap<String, String>) -> FetchResponse {
        FetchResponse {
            status,
            headers,
            body: Vec::new(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(50),
        }
    }

    #[test]
    fn test_accessible_200() {
        assert_eq!(
            classify_response(response(200, HashMap::new())),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_accessible_206() {
        assert_eq!(
            classify_response(response(206, HashMap::new())),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_accessible_other_status() {
        assert_eq!(
            classify_response(response(404, HashMap::new())),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_blocked_403() {
        assert_eq!(
            classify_response(response(403, HashMap::new())),
            ProbeResult::Blocked { status: 403 }
        );
    }

    #[test]
    fn test_blocked_503() {
        assert_eq!(
            classify_response(response(503, HashMap::new())),
            ProbeResult::Blocked { status: 503 }
        );
    }

    #[test]
    fn test_redirect_301() {
        let mut h = HashMap::new();
        h.insert("location".to_string(), "https://example.com/".to_string());
        assert_eq!(
            classify_response(response(301, h)),
            ProbeResult::Redirect {
                location: "https://example.com/".to_string(),
                status: 301
            }
        );
    }

    #[test]
    fn test_redirect_302_no_location() {
        assert_eq!(
            classify_response(response(302, HashMap::new())),
            ProbeResult::Redirect {
                location: String::new(),
                status: 302
            }
        );
    }

    #[test]
    fn test_redirect_307() {
        let mut h = HashMap::new();
        h.insert(
            "location".to_string(),
            "https://new.example.com".to_string(),
        );
        assert_eq!(
            classify_response(response(307, h)),
            ProbeResult::Redirect {
                location: "https://new.example.com".to_string(),
                status: 307
            }
        );
    }

    #[test]
    fn test_redirect_308() {
        let mut h = HashMap::new();
        h.insert(
            "location".to_string(),
            "https://perm.example.com".to_string(),
        );
        assert_eq!(
            classify_response(response(308, h)),
            ProbeResult::Redirect {
                location: "https://perm.example.com".to_string(),
                status: 308
            }
        );
    }

    #[test]
    fn test_error_timeout() {
        assert_eq!(
            classify_error(FetchError::Timeout),
            ProbeResult::Error {
                message: "probe timed out".to_string()
            }
        );
    }

    #[test]
    fn test_error_http_403() {
        assert_eq!(
            classify_error(FetchError::HttpError(403)),
            ProbeResult::Blocked { status: 403 }
        );
    }

    #[test]
    fn test_error_http_503() {
        assert_eq!(
            classify_error(FetchError::HttpError(503)),
            ProbeResult::Blocked { status: 503 }
        );
    }

    #[test]
    fn test_error_http_other() {
        assert_eq!(
            classify_error(FetchError::HttpError(500)),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_error_tls() {
        assert_eq!(
            classify_error(FetchError::TlsError("handshake failed".to_string())),
            ProbeResult::Error {
                message: "handshake failed".to_string()
            }
        );
    }

    #[test]
    fn test_error_network() {
        assert_eq!(
            classify_error(FetchError::NetworkError("connection refused".to_string())),
            ProbeResult::Error {
                message: "connection refused".to_string()
            }
        );
    }

    struct MockProber {
        response: Option<Result<FetchResponse, FetchError>>,
    }

    impl MockProber {
        fn with_ok(status: u16, headers: HashMap<String, String>) -> Self {
            Self {
                response: Some(Ok(response(status, headers))),
            }
        }

        fn with_err(e: FetchError) -> Self {
            Self {
                response: Some(Err(e)),
            }
        }
    }

    impl Prober for MockProber {
        fn probe(
            &self,
            _request: FetchRequest,
        ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, FetchError>> + Send + '_>> {
            Box::pin(std::future::ready(self.response.clone().unwrap()))
        }
    }

    #[tokio::test]
    async fn test_probe_domain_accessible() {
        let prober = MockProber::with_ok(200, HashMap::new());
        let result = probe_domain(&prober, "https://example.com").await;
        assert_eq!(result, ProbeResult::Accessible);
    }

    #[tokio::test]
    async fn test_probe_domain_blocked() {
        let prober = MockProber::with_ok(403, HashMap::new());
        let result = probe_domain(&prober, "https://blocked.com").await;
        assert_eq!(result, ProbeResult::Blocked { status: 403 });
    }

    #[tokio::test]
    async fn test_probe_domain_redirect() {
        let mut h = HashMap::new();
        h.insert("location".to_string(), "https://other.com".to_string());
        let prober = MockProber::with_ok(301, h);
        let result = probe_domain(&prober, "https://redirect.com").await;
        assert_eq!(
            result,
            ProbeResult::Redirect {
                location: "https://other.com".to_string(),
                status: 301
            }
        );
    }

    #[tokio::test]
    async fn test_probe_domain_timeout() {
        let prober = MockProber::with_err(FetchError::Timeout);
        let result = probe_domain(&prober, "https://slow.com").await;
        assert_eq!(
            result,
            ProbeResult::Error {
                message: "probe timed out".to_string()
            }
        );
    }

    #[tokio::test]
    async fn test_probe_domain_network_error() {
        let prober = MockProber::with_err(FetchError::NetworkError("dns failed".to_string()));
        let result = probe_domain(&prober, "https://down.com").await;
        assert_eq!(
            result,
            ProbeResult::Error {
                message: "dns failed".to_string()
            }
        );
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
            Box::pin(std::future::ready(result.unwrap_or(Err(
                FetchError::NetworkError("no mock response".to_string()),
            ))))
        }
    }

    fn ok_response(status: u16) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status,
            headers: HashMap::new(),
            body: b"ok".to_vec(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(100),
        })
    }

    #[tokio::test]
    async fn test_multi_probe_selects_first_successful() {
        let prober = MockStrategyProber::new()
            .respond_to("HttpSimple", Err(FetchError::Timeout))
            .respond_to("TlsChrome", ok_response(200));

        let strategies = vec![Strategy::HttpSimple, Strategy::TlsChrome];
        let result = probe_with_strategies(&prober, "example.com", &strategies).await;

        assert!(result.is_some());
        let winner = result.expect("should have a winner");
        assert_eq!(winner.strategy, Strategy::TlsChrome);
    }

    #[tokio::test]
    async fn test_multi_probe_all_fail_returns_none() {
        let prober = MockStrategyProber::new()
            .respond_to("HttpSimple", Err(FetchError::Timeout))
            .respond_to(
                "TlsChrome",
                Err(FetchError::NetworkError("fail".to_string())),
            )
            .respond_to(
                "Headless",
                Err(FetchError::BrowserError("crash".to_string())),
            );

        let strategies = vec![
            Strategy::HttpSimple,
            Strategy::TlsChrome,
            Strategy::Headless,
        ];
        let result = probe_with_strategies(&prober, "dead.com", &strategies).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_multi_probe_blocked_403_skipped() {
        let prober = MockStrategyProber::new()
            .respond_to(
                "HttpSimple",
                Ok(FetchResponse {
                    status: 403,
                    headers: HashMap::new(),
                    body: Vec::new(),
                    content_type: "text/html".to_string(),
                    latency: Duration::from_millis(50),
                }),
            )
            .respond_to("TlsChrome", ok_response(200));

        let strategies = vec![Strategy::HttpSimple, Strategy::TlsChrome];
        let result = probe_with_strategies(&prober, "blocked.com", &strategies).await;

        let winner = result.expect("should find TlsChrome");
        assert_eq!(winner.strategy, Strategy::TlsChrome);
    }

    #[tokio::test]
    async fn test_multi_probe_first_strategy_wins_when_fastest() {
        let prober = MockStrategyProber::new()
            .respond_to("HttpSimple", ok_response(200))
            .respond_to("TlsChrome", ok_response(200));

        let strategies = vec![Strategy::HttpSimple, Strategy::TlsChrome];
        let result = probe_with_strategies(&prober, "fast.com", &strategies).await;

        let winner = result.expect("should have a winner");
        assert_eq!(winner.strategy, Strategy::HttpSimple);
    }

    #[tokio::test]
    async fn test_probe_and_cache_caches_winner() {
        let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage"));
        let prober = MockStrategyProber::new()
            .respond_to("HttpSimple", Err(FetchError::Timeout))
            .respond_to("TlsChrome", ok_response(200))
            .respond_to("TlsMobile", ok_response(200))
            .respond_to("Headless", ok_response(200));

        let result = probe_and_cache(&prober, &storage, "newsite.com", &Strategy::HttpSimple).await;

        assert_eq!(result, Some(Strategy::TlsChrome));

        let cache = RouteCache::new(&storage);
        let cached = cache
            .get_best_strategy("newsite.com")
            .expect("cache lookup");
        assert!(cached.is_some());
        let cached = cached.expect("should exist");
        assert_eq!(cached.strategy, "TlsChrome");
        assert!(cached.success_rate > 0.0);
    }

    #[tokio::test]
    async fn test_probe_and_cache_all_fail_caches_default() {
        let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage"));
        let prober = MockStrategyProber::new()
            .respond_to("HttpSimple", Err(FetchError::Timeout))
            .respond_to(
                "TlsChrome",
                Err(FetchError::NetworkError("fail".to_string())),
            )
            .respond_to("TlsMobile", Err(FetchError::TlsError("fail".to_string())))
            .respond_to(
                "Headless",
                Err(FetchError::BrowserError("fail".to_string())),
            );

        let result =
            probe_and_cache(&prober, &storage, "deadzone.com", &Strategy::HttpSimple).await;

        assert_eq!(result, Some(Strategy::HttpSimple));

        let cache = RouteCache::new(&storage);
        let cached = cache
            .get_best_strategy("deadzone.com")
            .expect("cache lookup");
        assert!(cached.is_some());
        let cached = cached.expect("should exist");
        assert_eq!(cached.strategy, "HttpSimple");
        assert_eq!(cached.success_rate, 0.0);
    }

    #[test]
    fn test_is_probe_success_accepts_200() {
        let resp = FetchResponse {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(50),
        };
        assert!(is_probe_success(&resp));
    }

    #[test]
    fn test_is_probe_success_rejects_403() {
        let resp = FetchResponse {
            status: 403,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(50),
        };
        assert!(!is_probe_success(&resp));
    }

    #[test]
    fn test_is_probe_success_rejects_503() {
        let resp = FetchResponse {
            status: 503,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(50),
        };
        assert!(!is_probe_success(&resp));
    }

    #[test]
    fn test_is_probe_success_accepts_404() {
        let resp = FetchResponse {
            status: 404,
            headers: HashMap::new(),
            body: Vec::new(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(50),
        };
        assert!(is_probe_success(&resp));
    }
}
