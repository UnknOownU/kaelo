//! Integration tests for SPA wait-for-content feature.
//!
//! Run with: `cargo test --test spa_wait --all-features -- --ignored`

#![cfg(feature = "headless")]

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kaelo::types::FetchRequest;
use kaelo::fetch::backends::HeadlessBrowser;

fn make_request(url: &str, timeout: Duration) -> FetchRequest {
    FetchRequest {
        url: url.to_string(),
        headers: HashMap::new(),
        timeout,
        follow_redirects: true,
    }
}

/// Given a Blazor Server SPA (sbox.game), when fetching via HeadlessBrowser,
/// then the response contains real page content — not just the empty Blazor shell.
#[tokio::test]
#[ignore = "requires Chromium installed"]
async fn blazor_server_sbox_game() {
    let browser = HeadlessBrowser::new()
        .await
        .expect("HeadlessBrowser::new() should succeed");

    let request = make_request("https://sbox.game", Duration::from_secs(30));
    let result = browser.fetch_with_session(request, None).await;

    assert!(result.is_ok(), "fetch failed: {:?}", result.err());
    let resp = result.unwrap();
    let html = String::from_utf8_lossy(&resp.body);

    // Should NOT be the Blazor empty shell
    let text_content = html.replace(['<', '>'], "").trim().to_string();
    assert!(
        text_content.len() > 200,
        "Page content too short — SPA wait may not be working. Got {} chars",
        text_content.len()
    );

    // Should contain real s&box content
    assert!(
        html.contains("s&box") || html.contains("game") || html.contains("create"),
        "Expected real page content, got: {} chars",
        html.len()
    );
}

/// Given a static page (example.com), when fetching via HeadlessBrowser,
/// then the fast-path completes well under the timeout (no 2s+3s blind sleeps).
#[tokio::test]
#[ignore = "requires Chromium installed"]
async fn static_page_fast_path() {
    let browser = HeadlessBrowser::new()
        .await
        .expect("HeadlessBrowser::new() should succeed");

    let request = make_request("https://example.com", Duration::from_secs(15));
    let start = Instant::now();
    let result = browser.fetch_with_session(request, None).await;
    let elapsed = start.elapsed();

    assert!(result.is_ok(), "fetch failed: {:?}", result.err());

    // Fast-path should complete well under 10s (no 2s+3s blind sleeps)
    assert!(
        elapsed < Duration::from_secs(10),
        "Static page took too long ({:.1}s) — fast-path may not be working",
        elapsed.as_secs_f64()
    );
}

/// Given a non-routable IP (TEST-NET-1), when fetching via HeadlessBrowser,
/// then the request errors gracefully without panicking.
#[tokio::test]
#[ignore = "requires Chromium installed"]
async fn timeout_non_routable() {
    let browser = HeadlessBrowser::new()
        .await
        .expect("HeadlessBrowser::new() should succeed");

    let request = make_request("http://192.0.2.1", Duration::from_secs(5));
    let result = browser.fetch_with_session(request, None).await;

    // Should error (timeout), not panic
    assert!(
        result.is_err(),
        "Expected error for non-routable IP, got success"
    );
}
