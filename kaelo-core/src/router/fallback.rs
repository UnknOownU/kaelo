use std::collections::HashSet;

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

/// Strategy priority order for fallback.
pub fn fallback_order() -> Vec<Strategy> {
    vec![
        Strategy::HttpSimple,
        Strategy::TlsChrome,
        Strategy::TlsMobile,
        Strategy::Headless,
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

        let result = next_strategy(&Strategy::HttpSimple, &visited, 4, 1, &FetchError::Timeout);
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
            4,
            1,
            &FetchError::HttpError(404),
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_fallback_chain_state() {
        let mut chain = FallbackChain::new(4);
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
        assert!(chain.is_exhausted());

        // No more strategies left.
        let next = chain.next(&Strategy::Headless, &FetchError::Timeout);
        assert_eq!(next, None);
    }

    #[test]
    fn test_fallback_chain_stops_on_terminal() {
        let mut chain = FallbackChain::new(4);
        chain.record_attempt(&Strategy::HttpSimple);

        let next = chain.next(&Strategy::HttpSimple, &FetchError::HttpError(404));
        assert_eq!(next, None);
    }
}
