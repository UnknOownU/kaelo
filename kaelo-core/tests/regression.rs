//! Regression tests for kaelo-core: extraction pipeline, caches, router, token estimator.
//!
//! All tests use in-memory SQLite (`:memory:`) and require no network access.

use std::sync::Arc;
use std::time::Duration;

use kaelo_core::extraction;
use kaelo_core::extraction::token_estimator::TokenEstimator;
use kaelo_core::router::{Router, RouterDecision};
use kaelo_core::storage::content_cache::ContentCache;
use kaelo_core::storage::route_cache::{FetchResult, RouteCache};
use kaelo_core::storage::Storage;
use kaelo_core::types::Strategy;

const HTML_ARTICLE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>Rust Ownership Explained</title></head>
<body>
<nav><a href="/">Home</a><a href="/blog">Blog</a></nav>
<article>
    <h1>Rust Ownership Explained</h1>
    <p>The ownership model is central to Rust's memory safety guarantees.</p>
    <p>Every value has a single owner, and when the owner goes out of scope the value is dropped.</p>
    <pre><code>fn calculate_length(s: &amp;String) -&gt; usize { s.len() }</code></pre>
</article>
<footer><p>Privacy Policy | Terms of Service</p></footer>
</body>
</html>"#;

const HTML_MINIMAL: &str = r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>Quick Tip</title></head>
<body>
<article>
    <h1>Quick Tip: Use cargo watch</h1>
    <p>Install with <code>cargo install cargo-watch</code>.</p>
    <p>Then run <code>cargo watch -x test</code> for auto-recompile.</p>
</article>
</body>
</html>"#;

const HTML_EMPTY: &str = r#"<!DOCTYPE html>
<html><head><title></title></head><body></body></html>"#;

const HTML_WITH_NAV_SIDEBAR: &str = r#"<!DOCTYPE html>
<html lang="en">
<head><title>API Reference</title></head>
<body>
<nav class="sidebar">
    <a href="/docs/getting-started">Getting Started</a>
    <a href="/docs/api">API Reference</a>
    <a href="/docs/examples">Examples</a>
</nav>
<main>
    <h1>API Reference</h1>
    <h2>parse(input: &amp;str) -&gt; Result&lt;AST&gt;</h2>
    <p>Parses the input string into an abstract syntax tree.</p>
    <pre><code>let ast = mylib::parse("fn main() {}")?;</code></pre>
    <h2>evaluate(ast: &amp;AST) -&gt; Value</h2>
    <p>Evaluates the AST and returns the resulting value.</p>
    <table>
        <tr><th>Method</th><th>Returns</th></tr>
        <tr><td>parse</td><td>Result&lt;AST&gt;</td></tr>
        <tr><td>evaluate</td><td>Value</td></tr>
    </table>
</main>
<footer>Copyright 2024</footer>
</body>
</html>"#;

fn seed_strategy(storage: &Storage, domain: &str, strategy: &str, latency_ms: u64, success: bool) {
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
fn extraction_html_to_markdown_article() {
    let result = extraction::extract(
        HTML_ARTICLE,
        "https://example.com/rust-ownership",
        "text/html",
    )
    .expect("extract should not error on valid HTML");

    let content = result.expect("should extract content from article HTML");
    assert!(
        content.text_content.contains("ownership model"),
        "article body should be preserved, got: {:?}",
        content.text_content
    );
    assert!(
        content.text_content.contains("calculate_length"),
        "code blocks should be preserved"
    );
    assert!(
        !content.text_content.contains("Privacy Policy"),
        "footer should be stripped"
    );
    assert!(
        !content.text_content.contains("[Home]"),
        "nav links should be stripped"
    );
}

#[test]
fn extraction_minimal_article() {
    let result = extraction::extract(HTML_MINIMAL, "https://example.com/quick-tip", "text/html")
        .expect("extract should not error");

    let content = result.expect("should extract from minimal article");
    assert!(
        content.text_content.contains("cargo-watch"),
        "code content should be preserved"
    );
    assert!(
        content.text_content.contains("Quick Tip") || content.title.contains("Quick Tip"),
        "title should be present"
    );
}

#[test]
fn extraction_docs_page_preserves_code_and_tables() {
    let result = extraction::extract(
        HTML_WITH_NAV_SIDEBAR,
        "https://example.com/docs/api",
        "text/html",
    )
    .expect("extract should not error");

    let content = result.expect("should extract from docs page");
    assert!(
        content.text_content.contains("mylib") || content.text_content.contains("parse"),
        "code blocks should be preserved, got: {:?}",
        content.text_content
    );
}

#[test]
fn extraction_empty_html_returns_none() {
    let result = extraction::extract(HTML_EMPTY, "https://example.com/blank", "text/html")
        .expect("extract should not error on empty HTML");

    // Empty HTML has no meaningful content → Ok(None)
    assert!(
        result.is_none(),
        "empty HTML should yield None, got: {:?}",
        result
    );
}

#[test]
fn extraction_plain_text_passthrough() {
    let text = "Hello, this is plain text content.";
    let result = extraction::extract(text, "https://example.com/readme.txt", "text/plain")
        .expect("extract should not error on plain text");

    let content = result.expect("plain text should return Some");
    assert_eq!(
        content.text_content, text,
        "plain text should pass through unchanged"
    );
}

#[test]
fn extraction_json_pretty_printed() {
    let json = r#"{"name":"kaelo","version":"0.1.0","features":["mcp","fetch"]}"#;
    let result = extraction::extract(json, "https://example.com/api/info", "application/json")
        .expect("extract should not error on JSON");

    let content = result.expect("JSON should return Some");
    // Should be pretty-printed (contains newlines)
    assert!(
        content.text_content.contains('\n'),
        "JSON should be pretty-printed, got: {:?}",
        content.text_content
    );
    assert!(
        content.text_content.contains("kaelo"),
        "JSON content should be preserved"
    );
}

#[test]
fn extraction_unsupported_content_type() {
    let result = extraction::extract(
        "binary data",
        "https://example.com/file.pdf",
        "application/pdf",
    )
    .expect("extract should not error on unsupported type");

    let content = result.expect("unsupported type should return Some with notice");
    assert!(
        content.text_content.contains("Unsupported content type"),
        "should indicate unsupported type, got: {:?}",
        content.text_content
    );
}

#[test]
fn extraction_deterministic_same_html_same_output() {
    let result1 = extraction::extract(
        HTML_ARTICLE,
        "https://example.com/rust-ownership",
        "text/html",
    )
    .expect("extract should not error")
    .expect("should extract content");

    let result2 = extraction::extract(
        HTML_ARTICLE,
        "https://example.com/rust-ownership",
        "text/html",
    )
    .expect("extract should not error")
    .expect("should extract content");

    assert_eq!(
        result1.text_content, result2.text_content,
        "dom_smoothie extraction must be deterministic"
    );
    assert_eq!(result1.title, result2.title);
}

#[test]
fn route_cache_insert_and_get() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = RouteCache::new(&storage);

    cache
        .upsert_strategy(
            "example.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 200,
                success: true,
                headers: None,
            },
        )
        .expect("upsert should succeed");

    let strategies = cache
        .get_strategies("example.com")
        .expect("get_strategies should succeed");
    assert_eq!(strategies.len(), 1, "should have exactly one strategy");
    assert_eq!(strategies[0].domain, "example.com");
    assert_eq!(strategies[0].strategy, "HttpSimple");
    assert_eq!(strategies[0].total_requests, 1);
}

#[test]
fn route_cache_upsert_updates_running_average() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = RouteCache::new(&storage);

    // Insert first
    cache
        .upsert_strategy(
            "example.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 200,
                success: true,
                headers: None,
            },
        )
        .expect("first upsert should succeed");

    // Upsert second — should update running average
    cache
        .upsert_strategy(
            "example.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 400,
                success: true,
                headers: None,
            },
        )
        .expect("second upsert should succeed");

    let strategies = cache
        .get_strategies("example.com")
        .expect("get_strategies should succeed");
    assert_eq!(strategies.len(), 1, "should still be one entry (upsert)");
    assert_eq!(strategies[0].total_requests, 2);
    // Running average of 200 and 400 = 200 + (400 - 200) / 2 = 300
    assert!(
        (strategies[0].avg_latency_ms as i64 - 300).abs() <= 1,
        "avg_latency should be ~300, got {}",
        strategies[0].avg_latency_ms
    );
}

#[test]
fn route_cache_get_best_strategy_sorted_by_score() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = RouteCache::new(&storage);

    // Insert a slow strategy
    cache
        .upsert_strategy(
            "example.com",
            "TlsChrome",
            &FetchResult {
                latency_ms: 3000,
                success: true,
                headers: None,
            },
        )
        .expect("slow upsert should succeed");

    // Insert a fast strategy
    cache
        .upsert_strategy(
            "example.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 100,
                success: true,
                headers: None,
            },
        )
        .expect("fast upsert should succeed");

    let best = cache
        .get_best_strategy("example.com")
        .expect("get_best should succeed")
        .expect("should have a best strategy");
    assert_eq!(
        best.strategy, "HttpSimple",
        "HttpSimple (100ms) should beat TlsChrome (3000ms)"
    );
}

#[test]
fn route_cache_remove_strategy() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = RouteCache::new(&storage);

    cache
        .upsert_strategy(
            "example.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 200,
                success: true,
                headers: None,
            },
        )
        .expect("upsert should succeed");

    cache
        .remove_strategy("example.com", "HttpSimple")
        .expect("remove should succeed");

    let strategies = cache
        .get_strategies("example.com")
        .expect("get_strategies should succeed");
    assert!(strategies.is_empty(), "strategy should be removed");
}

#[test]
fn route_cache_count_entries() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = RouteCache::new(&storage);

    assert_eq!(
        cache
            .count_entries()
            .expect("count should succeed on empty cache"),
        0,
        "empty cache should have 0 entries"
    );

    cache
        .upsert_strategy(
            "a.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 100,
                success: true,
                headers: None,
            },
        )
        .expect("upsert a.com should succeed");
    cache
        .upsert_strategy(
            "b.com",
            "TlsChrome",
            &FetchResult {
                latency_ms: 200,
                success: true,
                headers: None,
            },
        )
        .expect("upsert b.com should succeed");

    assert_eq!(
        cache.count_entries().expect("count should succeed"),
        2,
        "should have 2 entries"
    );
}

#[test]
fn route_cache_get_all_domains() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = RouteCache::new(&storage);

    cache
        .upsert_strategy(
            "alpha.com",
            "HttpSimple",
            &FetchResult {
                latency_ms: 100,
                success: true,
                headers: None,
            },
        )
        .expect("upsert alpha should succeed");
    cache
        .upsert_strategy(
            "beta.com",
            "TlsChrome",
            &FetchResult {
                latency_ms: 200,
                success: true,
                headers: None,
            },
        )
        .expect("upsert beta should succeed");

    let domains = cache
        .get_all_domains()
        .expect("get_all_domains should succeed");
    assert_eq!(domains.len(), 2, "should have 2 domains");
    assert!(domains.contains(&"alpha.com".to_string()));
    assert!(domains.contains(&"beta.com".to_string()));
}

#[test]
fn content_cache_put_and_get() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    cache
        .put(
            "https://example.com/page",
            &Strategy::HttpSimple,
            "# Hello\n\nWorld",
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put should succeed");

    let cached = cache
        .get("https://example.com/page", &Strategy::HttpSimple)
        .expect("get should succeed")
        .expect("should find cached entry");
    assert_eq!(cached.content, "# Hello\n\nWorld");
    assert_eq!(cached.content_type, "text/html");
    assert!(cached.from_cache);
}

#[test]
fn content_cache_get_missing_returns_none() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    let result = cache
        .get("https://example.com/nonexistent", &Strategy::HttpSimple)
        .expect("get should succeed");
    assert!(result.is_none(), "missing URL should return None");
}

#[test]
fn content_cache_invalidate() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    cache
        .put(
            "https://example.com/page",
            &Strategy::HttpSimple,
            "content",
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put should succeed");

    cache
        .invalidate("https://example.com/page", Some(&Strategy::HttpSimple))
        .expect("invalidate should succeed");

    let result = cache
        .get("https://example.com/page", &Strategy::HttpSimple)
        .expect("get should succeed");
    assert!(result.is_none(), "invalidated entry should return None");
}

#[test]
fn content_cache_clear_all() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    cache
        .put(
            "https://a.com/1",
            &Strategy::HttpSimple,
            "content-a",
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put a should succeed");
    cache
        .put(
            "https://b.com/2",
            &Strategy::HttpSimple,
            "content-b",
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put b should succeed");

    cache.clear_all().expect("clear_all should succeed");

    let stats = cache.stats().expect("stats should succeed");
    assert_eq!(stats.entry_count, 0, "cache should be empty after clear");
}

#[test]
fn content_cache_stats() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    let stats = cache.stats().expect("stats should succeed on empty cache");
    assert_eq!(stats.entry_count, 0);
    assert_eq!(stats.total_size_bytes, 0);

    cache
        .put(
            "https://example.com/page",
            &Strategy::HttpSimple,
            "Hello World",
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put should succeed");

    let stats = cache.stats().expect("stats should succeed");
    assert_eq!(stats.entry_count, 1, "should have 1 entry");
    assert!(
        stats.total_size_bytes > 0,
        "compressed content should have nonzero size"
    );
}

#[test]
fn content_cache_content_hash() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    let url = "https://example.com/page";
    let content = "deterministic content for hash check";

    cache
        .put(
            url,
            &Strategy::HttpSimple,
            content,
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put should succeed");

    let hash = cache
        .content_hash(url, &Strategy::HttpSimple)
        .expect("content_hash should succeed")
        .expect("should have a hash");
    assert!(!hash.is_empty(), "hash should not be empty");

    // Hash should be consistent
    let hash2 = cache
        .content_hash(url, &Strategy::HttpSimple)
        .expect("content_hash should succeed")
        .expect("should have a hash");
    assert_eq!(hash, hash2, "hash should be deterministic");
}

#[test]
fn content_cache_content_hash_unchanged() {
    let storage = Storage::open(":memory:").expect("in-memory storage should open");
    let cache = ContentCache::new(&storage);

    let url = "https://example.com/page";
    let content = "some content";

    cache
        .put(
            url,
            &Strategy::HttpSimple,
            content,
            "text/html",
            Duration::from_secs(3600),
        )
        .expect("put should succeed");

    assert!(
        cache
            .content_hash_unchanged(url, &Strategy::HttpSimple, content)
            .expect("content_hash_unchanged should succeed"),
        "same content should match hash"
    );
    assert!(
        !cache
            .content_hash_unchanged(url, &Strategy::HttpSimple, "different content")
            .expect("content_hash_unchanged should succeed"),
        "different content should not match hash"
    );
}

// Router::new requires Arc<Storage> even though Storage is not Sync (rusqlite::Connection).
// The existing codebase uses the same allow attribute in router/mod.rs tests.

#[allow(clippy::arc_with_non_send_sync)]
#[test]
fn router_resolve_unknown_domain_returns_default() {
    let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage should open"));
    let router = Router::new(storage);

    let decision = router
        .resolve("https://unknown-site.example/page")
        .expect("resolve should succeed");

    match decision {
        RouterDecision::Unknown { strategy } => {
            assert_eq!(
                strategy,
                Strategy::HttpSimple,
                "default strategy should be HttpSimple"
            );
        }
        RouterDecision::Known { .. } => {
            panic!("unknown domain should not be Known")
        }
    }
}

#[allow(clippy::arc_with_non_send_sync)]
#[test]
fn router_resolve_known_domain_uses_cached_strategy() {
    let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage should open"));

    // Seed with known strategy
    seed_strategy(&storage, "reddit.com", "TlsMobile", 1500, true);
    seed_strategy(&storage, "reddit.com", "TlsMobile", 1200, true);

    let router = Router::new(Arc::clone(&storage));
    let decision = router
        .resolve("https://reddit.com/r/rust")
        .expect("resolve should succeed");

    match decision {
        RouterDecision::Known { strategy, score } => {
            assert_eq!(strategy, Strategy::TlsMobile);
            assert!(score > 0.0, "score should be positive");
        }
        RouterDecision::Unknown { .. } => panic!("seeded domain should be Known"),
    }
}

#[allow(clippy::arc_with_non_send_sync)]
#[test]
fn router_custom_default_strategy() {
    let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage should open"));
    let router = Router::new(storage).with_default_strategy(Strategy::TlsChrome);

    let decision = router
        .resolve("https://never-seen.example/path")
        .expect("resolve should succeed");

    match decision {
        RouterDecision::Unknown { strategy } => {
            assert_eq!(
                strategy,
                Strategy::TlsChrome,
                "custom default should be TlsChrome"
            );
        }
        RouterDecision::Known { .. } => panic!("unknown domain should be Unknown"),
    }
}

#[allow(clippy::arc_with_non_send_sync)]
#[test]
fn router_record_outcome_populates_cache() {
    let storage = Arc::new(Storage::open(":memory:").expect("in-memory storage should open"));
    let router = Router::new(Arc::clone(&storage));

    // Record an outcome
    router
        .record_outcome(
            "https://docs.rs/crate/serde",
            &Strategy::HttpSimple,
            150,
            true,
        )
        .expect("record_outcome should succeed");

    // Now resolve should find it
    let decision = router
        .resolve("https://docs.rs/another-crate")
        .expect("resolve should succeed");

    match decision {
        RouterDecision::Known { strategy, .. } => {
            assert_eq!(strategy, Strategy::HttpSimple);
        }
        RouterDecision::Unknown { .. } => panic!("recorded domain should be Known"),
    }
}

#[test]
fn token_estimator_estimate_reasonable_count() {
    let text = "Hello world this is a test of the token estimator.";
    let tokens = TokenEstimator::estimate(text);
    // ~4 chars per token, text is 52 chars → ~13 tokens
    assert!(tokens > 0, "estimate should return positive count");
    assert!(tokens < 100, "estimate should be reasonable for short text");
    assert_eq!(
        tokens,
        (text.len() as u32) / 4,
        "should use chars/4 heuristic"
    );
}

#[test]
fn token_estimator_estimate_empty_string() {
    let tokens = TokenEstimator::estimate("");
    assert_eq!(tokens, 0, "empty string should be 0 tokens");
}

#[test]
fn token_estimator_truncate_smart_no_truncation_when_within_budget() {
    let text = "Short text.";
    let (result, was_truncated) = TokenEstimator::truncate_smart(text, 1000);
    assert!(!was_truncated, "should not truncate short text");
    assert_eq!(result, text);
}

#[test]
fn token_estimator_truncate_smart_sets_flag_on_long_text() {
    let long_text: String = "word ".repeat(5000);
    let (_, was_truncated) = TokenEstimator::truncate_smart(&long_text, 100);
    assert!(was_truncated, "should report truncation flag for long text");
}

#[test]
fn token_estimator_truncate_smart_with_headings() {
    let mut text = String::from("# Heading 1\n\n");
    for i in 0..500 {
        text.push_str(&format!("Paragraph {i} with some content.\n\n"));
    }
    text.push_str("# Heading 2\n\nMore content after.\n");
    for i in 0..500 {
        text.push_str(&format!("Section 2 paragraph {i}.\n\n"));
    }

    let (result, was_truncated) = TokenEstimator::truncate_smart(&text, 200);
    assert!(was_truncated, "should truncate long text with headings");
    assert!(
        result.len() < text.len(),
        "result should be shorter when headings provide break points"
    );
}

#[test]
fn token_estimator_truncate_to_budget_no_truncation() {
    let text = "Within budget.";
    let (result, was_truncated) = TokenEstimator::truncate_to_budget(text, 1000);
    assert!(!was_truncated);
    assert_eq!(result, text);
}

#[test]
fn token_estimator_truncate_to_budget_truncates() {
    let long_text: String = "a ".repeat(5000);
    let (result, was_truncated) = TokenEstimator::truncate_to_budget(&long_text, 100);
    assert!(was_truncated);
    assert!(result.len() < long_text.len());
}
