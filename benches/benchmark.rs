use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use kaelo::storage::route_cache::FetchResult;

fn benchmark_extraction(c: &mut Criterion) {
    let html = include_str!("../tests/fixtures/blog_post.html");
    c.bench_function("extract_blog_post", |b| {
        b.iter(|| kaelo::extraction::extract(html, "https://example.com", "text/html"))
    });

    let news_html = include_str!("../tests/fixtures/news_article.html");
    c.bench_function("extract_news_article", |b| {
        b.iter(|| {
            kaelo::extraction::extract(news_html, "https://example.com/news", "text/html")
        })
    });

    let docs_html = include_str!("../tests/fixtures/docs_page.html");
    c.bench_function("extract_docs_page", |b| {
        b.iter(|| {
            kaelo::extraction::extract(docs_html, "https://example.com/docs", "text/html")
        })
    });
}

fn benchmark_cold_start(c: &mut Criterion) {
    c.bench_function("cold_start_storage", |b| {
        b.iter(|| {
            let storage = kaelo::storage::Storage::open(":memory:").unwrap();
            drop(storage);
        })
    });
}

fn benchmark_warm_cache(c: &mut Criterion) {
    let storage = kaelo::storage::Storage::open(":memory:").unwrap();
    let rc = kaelo::storage::route_cache::RouteCache::new(&storage);
    for i in 0..50 {
        let domain = format!("example{}.com", i);
        let result = FetchResult {
            latency_ms: 150,
            success: true,
            headers: None,
        };
        rc.upsert_strategy(&domain, "HttpSimple", &result).unwrap();
    }

    c.bench_function("warm_cache_lookup", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let domain = format!("example{}.com", i % 50);
            let _ = rc.get_best_strategy(&domain);
            i += 1;
        })
    });
}

fn benchmark_sqlite_ops(c: &mut Criterion) {
    let storage = kaelo::storage::Storage::open(":memory:").unwrap();
    let cc = kaelo::storage::content_cache::ContentCache::new(&storage);
    use kaelo::types::Strategy;

    c.bench_function("content_cache_store", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let url = format!("https://example.com/page{}", i % 100);
            let content = "Lorem ipsum ".repeat(100);
            cc.put(
                &url,
                &Strategy::HttpSimple,
                &content,
                "text/html",
                Duration::from_secs(3600),
            )
            .unwrap();
            i += 1;
        })
    });

    for i in 0..100 {
        let url = format!("https://example.com/page{}", i);
        let content = "Lorem ipsum ".repeat(100);
        cc.put(
            &url,
            &Strategy::HttpSimple,
            &content,
            "text/html",
            Duration::from_secs(3600),
        )
        .unwrap();
    }

    c.bench_function("content_cache_retrieve", |b| {
        let mut i = 0u64;
        b.iter(|| {
            let url = format!("https://example.com/page{}", i % 100);
            cc.get(&url, &Strategy::HttpSimple).ok();
            i += 1;
        })
    });
}

criterion_group!(extraction_benches, benchmark_extraction);
criterion_group!(cold_start_benches, benchmark_cold_start);
criterion_group!(warm_cache_benches, benchmark_warm_cache);
criterion_group!(sqlite_benches, benchmark_sqlite_ops);
criterion_main!(
    extraction_benches,
    cold_start_benches,
    warm_cache_benches,
    sqlite_benches
);
