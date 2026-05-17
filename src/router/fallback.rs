use std::collections::{HashMap, HashSet};

use crate::block_detect::{self, DetectedBlock};
use crate::types::{FetchError, Strategy};

/// Classification of fetch errors for fallback decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorClass {
    /// Network/timeout — try next backend.
    Transient,
    /// TLS/blocking — try TLS impersonation or headless.
    Blocking,
    /// 404/not found — don't retry, the page doesn't exist.
    NotFound,
    /// Rate limited — back off, don't retry immediately.
    RateLimited,
    /// Anti-bot block detected (Cloudflare, PerimeterX, etc.).
    Blocked(crate::types::BlockType),
}

/// Classify a [`FetchError`] into an [`ErrorClass`] for fallback logic.
pub fn classify_error(error: &FetchError) -> ErrorClass {
    match error {
        FetchError::Timeout => ErrorClass::Transient,
        FetchError::HttpError(404) => ErrorClass::NotFound,
        FetchError::HttpError(429) => ErrorClass::RateLimited,
        FetchError::HttpError(403) | FetchError::HttpError(503) => ErrorClass::Blocking,
        FetchError::TlsError(_) => ErrorClass::Transient,
        FetchError::BrowserError(_) => ErrorClass::Transient,
        FetchError::NetworkError(_) => ErrorClass::Transient,
        FetchError::HttpError(_) => ErrorClass::NotFound,
    }
}

/// Classify a response using full body + headers via block_detect.
///
/// This provides richer classification than [`classify_error`] by inspecting
/// response content to identify specific blockers (Cloudflare, PerimeterX, etc.).
/// Returns `None` if no block is detected.
pub fn classify_response_block(
    status: u16,
    body: &str,
    headers: &HashMap<String, String>,
) -> Option<ErrorClass> {
    let detected = block_detect::classify_response(status, body, headers)?;
    Some(match detected {
        DetectedBlock::RateLimit => ErrorClass::RateLimited,
        DetectedBlock::AuthWall => ErrorClass::NotFound,
        other => {
            let block_type = other.to_block_type();
            tracing::info!(block_type = ?block_type, status, "Anti-bot block detected during fallback");
            ErrorClass::Blocked(block_type)
        }
    })
}

/// Strategy priority order for fallback.
pub fn fallback_order() -> Vec<Strategy> {
    vec![
        Strategy::HttpSimple,
        Strategy::TlsChrome,
        Strategy::TlsMobile,
        Strategy::Headless,
        Strategy::PublicApi,
        Strategy::YouTube,
    ]
}

/// Determine the next strategy to try after a failure.
///
/// Returns `None` if:
/// - all strategies exhausted (visited or no more in order),
/// - `max_attempts` reached, or
/// - the error is terminal (`NotFound`).
pub fn next_strategy(
    current: &Strategy,
    visited: &HashSet<Strategy>,
    max_attempts: u32,
    attempts: u32,
    error: &FetchError,
) -> Option<Strategy> {
    if attempts >= max_attempts {
        return None;
    }

    if matches!(classify_error(error), ErrorClass::NotFound) {
        return None;
    }

    let order = fallback_order();
    let current_idx = order.iter().position(|s| s == current).unwrap_or(0);

    for (idx, strategy) in order.iter().enumerate() {
        if idx > current_idx && !visited.contains(strategy) {
            return Some(strategy.clone());
        }
    }

    None
}

/// Tracks state across retry attempts for a single fetch operation.
pub struct FallbackChain {
    max_attempts: u32,
    visited: HashSet<Strategy>,
    attempts: u32,
}

impl FallbackChain {
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            visited: HashSet::new(),
            attempts: 0,
        }
    }

    pub fn record_attempt(&mut self, strategy: &Strategy) {
        self.visited.insert(strategy.clone());
        self.attempts += 1;
    }

    pub fn next(&self, current: &Strategy, error: &FetchError) -> Option<Strategy> {
        next_strategy(
            current,
            &self.visited,
            self.max_attempts,
            self.attempts,
            error,
        )
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub fn is_exhausted(&self) -> bool {
        self.attempts >= self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_timeout_transient() {
        assert_eq!(classify_error(&FetchError::Timeout), ErrorClass::Transient);
    }

    #[test]
    fn test_classify_404_not_found() {
        assert_eq!(
            classify_error(&FetchError::HttpError(404)),
            ErrorClass::NotFound
        );
    }

    #[test]
    fn test_classify_403_blocking() {
        assert_eq!(
            classify_error(&FetchError::HttpError(403)),
            ErrorClass::Blocking
        );
    }

    #[test]
    fn test_classify_503_blocking() {
        assert_eq!(
            classify_error(&FetchError::HttpError(503)),
            ErrorClass::Blocking
        );
    }

    #[test]
    fn test_classify_429_rate_limited() {
        assert_eq!(
            classify_error(&FetchError::HttpError(429)),
            ErrorClass::RateLimited
        );
    }

    #[test]
    fn test_classify_tls_error_transient() {
        assert_eq!(
            classify_error(&FetchError::TlsError("handshake failed".into())),
            ErrorClass::Transient
        );
    }

    #[test]
    fn test_classify_browser_error_transient() {
        assert_eq!(
            classify_error(&FetchError::BrowserError("crash".into())),
            ErrorClass::Transient
        );
    }

    #[test]
    fn test_classify_network_error_transient() {
        assert_eq!(
            classify_error(&FetchError::NetworkError("connection reset".into())),
            ErrorClass::Transient
        );
    }

    #[test]
    fn test_classify_other_http_not_found() {
        // Other HTTP errors are treated as terminal / not-found.
        assert_eq!(
            classify_error(&FetchError::HttpError(500)),
            ErrorClass::NotFound
        );
    }

    #[test]
    fn test_next_strategy_advances() {
        let visited = HashSet::new();
        let result = next_strategy(&Strategy::HttpSimple, &visited, 4, 1, &FetchError::Timeout);
        assert_eq!(result, Some(Strategy::TlsChrome));
    }

    #[test]
    fn test_next_strategy_skips_visited() {
        let mut visited = HashSet::new();
        visited.insert(Strategy::TlsChrome);

        let result = next_strategy(&Strategy::HttpSimple, &visited, 4, 1, &FetchError::Timeout);
        assert_eq!(result, Some(Strategy::TlsMobile));
    }

    #[test]
    fn test_next_strategy_exhausted() {
        let mut visited = HashSet::new();
        visited.insert(Strategy::TlsChrome);
        visited.insert(Strategy::TlsMobile);
        visited.insert(Strategy::Headless);
        visited.insert(Strategy::PublicApi);
        visited.insert(Strategy::YouTube);

        let result = next_strategy(&Strategy::HttpSimple, &visited, 5, 1, &FetchError::Timeout);
        assert_eq!(result, None);
    }

    #[test]
    fn test_next_strategy_max_attempts() {
        let visited = HashSet::new();
        let result = next_strategy(
            &Strategy::HttpSimple,
            &visited,
            4,
            4, // already at max
            &FetchError::Timeout,
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_next_strategy_terminal_error() {
        let visited = HashSet::new();
        let result = next_strategy(
            &Strategy::HttpSimple,
            &visited,
            5,
            1,
            &FetchError::HttpError(404),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_fallback_chain_state() {
        let mut chain = FallbackChain::new(5);
        assert_eq!(chain.attempts(), 0);
        assert!(!chain.is_exhausted());

        chain.record_attempt(&Strategy::HttpSimple);
        assert_eq!(chain.attempts(), 1);
        assert!(!chain.is_exhausted());

        let next = chain.next(&Strategy::HttpSimple, &FetchError::Timeout);
        assert_eq!(next, Some(Strategy::TlsChrome));

        chain.record_attempt(&Strategy::TlsChrome);
        assert_eq!(chain.attempts(), 2);

        let next = chain.next(&Strategy::TlsChrome, &FetchError::Timeout);
        assert_eq!(next, Some(Strategy::TlsMobile));

        chain.record_attempt(&Strategy::TlsMobile);
        chain.record_attempt(&Strategy::Headless);
        assert_eq!(chain.attempts(), 4);

        // PublicApi is available as the last resort.
        let next = chain.next(&Strategy::Headless, &FetchError::Timeout);
        assert_eq!(next, Some(Strategy::PublicApi));

        chain.record_attempt(&Strategy::PublicApi);
        assert_eq!(chain.attempts(), 5);
        assert!(chain.is_exhausted());

        // No more strategies left.
        let next = chain.next(&Strategy::PublicApi, &FetchError::Timeout);
        assert_eq!(next, None);
    }

    #[test]
    fn test_fallback_chain_stops_on_terminal() {
        let mut chain = FallbackChain::new(4);
        chain.record_attempt(&Strategy::HttpSimple);

        let next = chain.next(&Strategy::HttpSimple, &FetchError::HttpError(404));
        assert_eq!(next, None);
    }

    fn make_headers(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn test_classify_response_block_cloudflare() {
        let headers = make_headers(&[("cf-ray", "abc123")]);
        let result = classify_response_block(403, "Access denied", &headers);
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::Cloudflare))
        );
    }

    #[test]
    fn test_classify_response_block_perimeterx() {
        let headers = make_headers(&[]);
        let result = classify_response_block(403, "window._pxCaptcha = '...'", &headers);
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::PerimeterX))
        );
    }

    #[test]
    fn test_classify_response_block_datadome() {
        let headers = make_headers(&[("set-cookie", "datadome=abc")]);
        let result = classify_response_block(403, "blocked", &headers);
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::DataDome))
        );
    }

    #[test]
    fn test_classify_response_block_akamai() {
        let headers = make_headers(&[("x-akamai-transformed", "1")]);
        let result = classify_response_block(403, "access denied", &headers);
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::Akamai))
        );
    }

    #[test]
    fn test_classify_response_block_captcha() {
        let headers = make_headers(&[]);
        let result = classify_response_block(403, "please complete the recaptcha", &headers);
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::Captcha))
        );
    }

    #[test]
    fn test_classify_response_block_unknown_403() {
        let headers = make_headers(&[]);
        let result = classify_response_block(403, "generic access denied", &headers);
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::Unknown))
        );
    }

    #[test]
    fn test_classify_response_block_rate_limit() {
        let headers = make_headers(&[]);
        let result = classify_response_block(429, "slow down", &headers);
        assert_eq!(result, Some(ErrorClass::RateLimited));
    }

    #[test]
    fn test_classify_response_block_auth_wall() {
        let headers = make_headers(&[]);
        let result = classify_response_block(401, "login required", &headers);
        assert_eq!(result, Some(ErrorClass::NotFound));
    }

    #[test]
    fn test_classify_response_block_paywall() {
        let headers = make_headers(&[]);
        let result = classify_response_block(
            200,
            "subscribe to continue reading this premium content",
            &headers,
        );
        assert_eq!(
            result,
            Some(ErrorClass::Blocked(crate::types::BlockType::Paywall))
        );
    }

    #[test]
    fn test_classify_response_block_no_block() {
        let headers = make_headers(&[]);
        let result = classify_response_block(200, "normal page content", &headers);
        assert_eq!(result, None);
    }

    #[test]
    fn test_classify_response_block_500_no_block() {
        let headers = make_headers(&[]);
        let result = classify_response_block(500, "internal server error", &headers);
        assert_eq!(result, None);
    }

    #[test]
    fn test_classify_error_unchanged() {
        assert_eq!(
            classify_error(&FetchError::HttpError(403)),
            ErrorClass::Blocking
        );
        assert_eq!(
            classify_error(&FetchError::HttpError(503)),
            ErrorClass::Blocking
        );
        assert_eq!(
            classify_error(&FetchError::HttpError(404)),
            ErrorClass::NotFound
        );
        assert_eq!(
            classify_error(&FetchError::HttpError(429)),
            ErrorClass::RateLimited
        );
        assert_eq!(classify_error(&FetchError::Timeout), ErrorClass::Transient);
    }

    #[test]
    fn test_next_strategy_allows_retry_on_blocking() {
        let visited = HashSet::new();
        let result = next_strategy(
            &Strategy::HttpSimple,
            &visited,
            5,
            1,
            &FetchError::HttpError(403),
        );
        assert_eq!(result, Some(Strategy::TlsChrome));
    }

    #[test]
    fn test_fallback_chain_retries_on_block() {
        let mut chain = FallbackChain::new(5);
        chain.record_attempt(&Strategy::HttpSimple);
        let next = chain.next(&Strategy::HttpSimple, &FetchError::HttpError(403));
        assert_eq!(next, Some(Strategy::TlsChrome));
    }
}
