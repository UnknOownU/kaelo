//! Stress tests for the browser pool.
//!
//! All tests require a Chromium installation and are behind `#[ignore]`.
//! Run with: `cargo test --test browser_stress --all-features -- --ignored`

use std::collections::HashMap;
use std::time::{Duration, Instant};

use kaelo::types::FetchRequest;
use kaelo::fetch::backend::FetchBackend;
use kaelo::fetch::backends::{shutdown_browser_pool, HeadlessBrowser};

fn make_request(url: &str) -> FetchRequest {
    FetchRequest {
        url: url.to_string(),
        headers: HashMap::new(),
        timeout: Duration::from_secs(30),
        follow_redirects: true,
    }
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn sequential_50_fetches_no_panic_or_oom() {
    let browser = HeadlessBrowser::new()
        .await
        .expect("HeadlessBrowser::new() should succeed");

    let url = "https://example.com";
    let mut latencies: Vec<Duration> = Vec::with_capacity(50);

    for i in 0..50 {
        let start = Instant::now();
        let result = browser.fetch(make_request(url)).await;
        let elapsed = start.elapsed();

        assert!(
            result.is_ok(),
            "request {} failed: {:?}",
            i + 1,
            result.err()
        );
        latencies.push(elapsed);
    }

    let first = latencies[0];
    let rest_avg = latencies[1..].iter().sum::<Duration>() / (latencies.len() - 1) as u32;

    eprintln!(
        "sequential stress: first={:.2}s, avg(rest)={:.2}s",
        first.as_secs_f64(),
        rest_avg.as_secs_f64()
    );

    assert!(
        rest_avg <= first * 5,
        "pool-warm avg ({:.2}s) unexpectedly slow vs cold start ({:.2}s)",
        rest_avg.as_secs_f64(),
        first.as_secs_f64(),
    );

    shutdown_browser_pool().await;
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn concurrent_10_fetches_all_succeed() {
    let _warmer = HeadlessBrowser::new()
        .await
        .expect("pool warmup before concurrent tasks");

    let urls = [
        "https://example.com",
        "https://www.example.com",
        "https://example.com/",
        "https://example.com/#section",
        "https://httpbin.org/get",
        "https://httpbin.org/ip",
        "https://httpbin.org/headers",
        "https://docs.rs",
        "https://docs.rs/tokio",
        "https://docs.rs/serde",
    ];

    let mut handles = Vec::with_capacity(urls.len());

    for url in urls {
        let br = HeadlessBrowser::new()
            .await
            .expect("HeadlessBrowser::new() per concurrent handle");
        let req = make_request(url);
        handles.push(tokio::spawn(async move {
            let start = Instant::now();
            let result = br.fetch(req).await;
            (url, result, start.elapsed())
        }));
    }

    let mut all_ok = true;
    for handle in handles {
        let (url, result, elapsed) = handle.await.expect("task should not panic");
        match result {
            Ok(resp) => {
                eprintln!(
                    "  concurrent: {} — {} in {:.2}s",
                    url,
                    resp.status,
                    elapsed.as_secs_f64()
                );
                assert_eq!(resp.status, 200, "expected 200 for {url}");
            }
            Err(e) => {
                eprintln!("  concurrent: {url} — FAILED: {e}");
                all_ok = false;
            }
        }
    }

    assert!(all_ok, "at least one concurrent fetch failed");

    shutdown_browser_pool().await;
}

#[tokio::test]
#[ignore = "requires Chromium"]
async fn crash_recovery_after_pool_shutdown() {
    let browser = HeadlessBrowser::new()
        .await
        .expect("HeadlessBrowser::new() phase 1");

    let result = browser.fetch(make_request("https://example.com")).await;
    assert!(result.is_ok(), "phase 1 fetch: {:?}", result.err());
    eprintln!(
        "  crash recovery: phase 1 OK (status={})",
        result.expect("checked").status
    );

    shutdown_browser_pool().await;
    eprintln!("  crash recovery: pool shut down");

    let browser2 = HeadlessBrowser::new()
        .await
        .expect("HeadlessBrowser::new() phase 3");

    let result2 = browser2.fetch(make_request("https://example.com")).await;
    assert!(
        result2.is_ok(),
        "phase 3 fetch after shutdown should recreate browser: {:?}",
        result2.err()
    );
    eprintln!(
        "  crash recovery: phase 3 OK (status={})",
        result2.expect("checked").status
    );

    shutdown_browser_pool().await;
}
