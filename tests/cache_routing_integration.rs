//! Integration tests for cache + routing fix.
//!
//! Verifies strategy-aware caching, fallback logic, and quality gates.
//! All tests use in-memory SQLite and require no network access.

use std::collections::HashSet;
use std::time::Duration;

use kaelo::extraction::quality::is_content_valuable;
use kaelo::router::fallback::{classify_error, next_strategy, ErrorClass};
use kaelo::storage::content_cache::ContentCache;
use kaelo::storage::Storage;
use kaelo::types::{FetchError, Strategy};

#[test]
fn test_different_strategies_different_cache_entries() {
    let storage = Storage::open(":memory:").unwrap();
    let cache = ContentCache::new(&storage);

    // Store same URL with different strategies
    cache
        .put(
            "https://example.com",
            &Strategy::TlsChrome,
            "TlsChrome content",
            "text/html",
            Duration::from_secs(3600),
        )
        .unwrap();
    cache
        .put(
            "https://example.com",
            &Strategy::Headless,
            "Headless content",
            "text/html",
            Duration::from_secs(3600),
        )
        .unwrap();

    // Each strategy gets its own entry
    let tls = cache
        .get("https://example.com", &Strategy::TlsChrome)
        .unwrap()
        .unwrap();
    assert_eq!(tls.content, "TlsChrome content");

    let headless = cache
        .get("https://example.com", &Strategy::Headless)
        .unwrap()
        .unwrap();
    assert_eq!(headless.content, "Headless content");

    // Non-cached strategy returns None
    let simple = cache
        .get("https://example.com", &Strategy::HttpSimple)
        .unwrap();
    assert!(simple.is_none());
}

#[test]
fn test_get_any_returns_best_available() {
    let storage = Storage::open(":memory:").unwrap();
    let cache = ContentCache::new(&storage);

    cache
        .put(
            "https://example.com",
            &Strategy::Headless,
            "Headless content",
            "text/html",
            Duration::from_secs(3600),
        )
        .unwrap();

    // get_any returns content regardless of strategy
    let result = cache.get_any("https://example.com").unwrap().unwrap();
    assert_eq!(result.content, "Headless content");

    // get_any returns None for unknown URL
    let missing = cache.get_any("https://unknown.com").unwrap();
    assert!(missing.is_none());
}

#[test]
fn test_invalidate_specific_strategy() {
    let storage = Storage::open(":memory:").unwrap();
    let cache = ContentCache::new(&storage);

    cache
        .put(
            "https://example.com",
            &Strategy::TlsChrome,
            "TlsChrome content",
            "text/html",
            Duration::from_secs(3600),
        )
        .unwrap();
    cache
        .put(
            "https://example.com",
            &Strategy::Headless,
            "Headless content",
            "text/html",
            Duration::from_secs(3600),
        )
        .unwrap();

    // Invalidate only TlsChrome
    cache
        .invalidate("https://example.com", Some(&Strategy::TlsChrome))
        .unwrap();

    // TlsChrome gone, Headless still there
    assert!(cache
        .get("https://example.com", &Strategy::TlsChrome)
        .unwrap()
        .is_none());
    assert!(cache
        .get("https://example.com", &Strategy::Headless)
        .unwrap()
        .is_some());

    // Invalidate all
    cache.invalidate("https://example.com", None).unwrap();
    assert!(cache
        .get("https://example.com", &Strategy::Headless)
        .unwrap()
        .is_none());
}

#[test]
fn test_quality_validation_rejects_empty() {
    assert!(!is_content_valuable(""));
    assert!(!is_content_valuable("<div id=\"app\"></div>"));
    assert!(!is_content_valuable("short"));
    assert!(is_content_valuable(
        "This is a paragraph with enough content to meet the new minimum threshold.\n\n\
         Second paragraph with additional text to pad the character count.\n\n\
         Third paragraph with even more content to ensure we exceed 200 characters total.\n\n\
         Fourth paragraph to also meet the five non-empty line requirement.\n\n\
         Fifth paragraph confirming the content is substantial enough to cache."
    ));
}

#[test]
fn test_fallback_skips_terminal_errors() {
    // 404 is terminal — no fallback
    let error_404 = FetchError::HttpError(404);
    assert_eq!(classify_error(&error_404), ErrorClass::NotFound);
    assert!(next_strategy(&Strategy::TlsChrome, &HashSet::new(), 3, 1, &error_404).is_none());

    // 403 triggers fallback
    let error_403 = FetchError::HttpError(403);
    assert_eq!(classify_error(&error_403), ErrorClass::Blocking);
    let next = next_strategy(&Strategy::TlsChrome, &HashSet::new(), 3, 1, &error_403);
    assert!(next.is_some());
    assert_eq!(next.unwrap(), Strategy::TlsMobile);

    // 429 is rate-limited (not terminal for fallback — it still tries next strategy)
    let error_429 = FetchError::HttpError(429);
    assert_eq!(classify_error(&error_429), ErrorClass::RateLimited);
    let next_429 = next_strategy(&Strategy::TlsChrome, &HashSet::new(), 3, 1, &error_429);
    assert!(next_429.is_some());
    assert_eq!(next_429.unwrap(), Strategy::TlsMobile);

    // Any other HTTP error is classified as NotFound (terminal)
    let error_500 = FetchError::HttpError(500);
    assert_eq!(classify_error(&error_500), ErrorClass::NotFound);
    assert!(next_strategy(&Strategy::TlsChrome, &HashSet::new(), 3, 1, &error_500).is_none());
}
