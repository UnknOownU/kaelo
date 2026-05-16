//! Backend tests for kaelo-fetch: HttpSimple, HTML fixture extraction, TLS smoke.
//!
//! Network-dependent tests are behind `#[ignore]` so `cargo test` works offline.
//! Run all including ignored: `cargo test -p kaelo-fetch -- --ignored`

use std::collections::HashMap;
use std::time::Duration;

use kaelo_core::types::{FetchError, FetchRequest, Strategy};
use kaelo_fetch::backend::FetchBackend;
use kaelo_fetch::backends::HttpSimple;

const HTML_ARTICLE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>Test Article</title></head>
<body>
<nav><a href="/">Home</a></nav>
<article>
    <h1>Test Article Title</h1>
    <p>This is the first paragraph of the article content.</p>
    <p>This is the second paragraph with <strong>bold text</strong>.</p>
    <pre><code>fn main() { println!("hello"); }</code></pre>
</article>
<footer><p>&copy; 2024 Test Corp</p></footer>
</body>
</html>"#;

const HTML_MINIMAL: &str = r#"<!DOCTYPE html>
<html>
<head><title>Minimal</title></head>
<body><p>Hello World</p></body>
</html>"#;

fn make_request(url: &str) -> FetchRequest {
    FetchRequest {
        url: url.to_string(),
        headers: HashMap::new(),
        timeout: Duration::from_secs(10),
        follow_redirects: true,
    }
}

#[test]
fn http_simple_new_creates_client() {
    let backend = HttpSimple::new();
    assert!(
        backend.is_ok(),
        "HttpSimple::new() should succeed: {:?}",
        backend.err()
    );
}

#[test]
fn http_simple_name_returns_correct_strategy() {
    let backend = HttpSimple::new().expect("HttpSimple::new should succeed");
    assert_eq!(backend.name(), Strategy::HttpSimple);
}

#[tokio::test]
#[ignore = "requires network"]
async fn http_simple_fetch_example_com() {
    let backend = HttpSimple::new().expect("HttpSimple::new should succeed");
    let response = backend.fetch(make_request("https://example.com")).await;

    assert!(
        response.is_ok(),
        "fetch example.com should succeed: {:?}",
        response.err()
    );
    let resp = response.expect("already checked");
    assert_eq!(resp.status, 200, "example.com should return 200");
    assert!(!resp.body.is_empty(), "response body should not be empty");
    assert!(
        resp.content_type.contains("text/html"),
        "content-type should be text/html, got: {:?}",
        resp.content_type
    );
}

#[tokio::test]
#[ignore = "requires network"]
async fn http_simple_fetch_returns_headers() {
    let backend = HttpSimple::new().expect("HttpSimple::new should succeed");
    let response = backend
        .fetch(make_request("https://example.com"))
        .await
        .expect("fetch should succeed");

    assert!(!response.headers.is_empty(), "response should have headers");
}

#[tokio::test]
#[ignore = "requires network"]
async fn http_simple_404_returns_http_error() {
    let backend = HttpSimple::new().expect("HttpSimple::new should succeed");
    let result = backend
        .fetch(make_request("https://httpbin.org/status/404"))
        .await;

    match result {
        Err(FetchError::HttpError(404)) => {}
        Err(FetchError::HttpError(code)) => {
            panic!("expected HTTP 404, got {code}");
        }
        Err(other) => {
            panic!("expected HttpError, got: {other}");
        }
        Ok(resp) => {
            assert!(
                resp.status >= 400,
                "expected error status, got {}",
                resp.status
            );
        }
    }
}

#[test]
fn html_fixture_extraction_article() {
    use kaelo_core::extraction;

    let result = extraction::extract(HTML_ARTICLE, "https://example.com/article", "text/html")
        .expect("extract should not error");

    let content = result.expect("should extract content from article fixture");
    assert!(
        content.text_content.contains("first paragraph"),
        "article body should be preserved"
    );
    assert!(
        content.text_content.contains("fn main"),
        "code blocks should be preserved"
    );
    assert!(
        !content.text_content.contains("Test Corp"),
        "footer should be stripped"
    );
}

#[test]
fn html_fixture_extraction_minimal() {
    use kaelo_core::extraction;

    let result = extraction::extract(HTML_MINIMAL, "https://example.com/min", "text/html")
        .expect("extract should not error");

    let content = result.expect("should extract from minimal HTML");
    assert!(
        content.text_content.contains("Hello World"),
        "minimal content should be preserved"
    );
}

#[cfg(feature = "tls-impersonation")]
mod tls_impersonation_smoke {
    use kaelo_fetch::backends::TlsImpersonation;

    #[test]
    fn tls_impersonation_backend_compiles() {
        let backend = TlsImpersonation::new();
        assert!(
            backend.is_ok(),
            "TlsImpersonation::new() should succeed: {:?}",
            backend.err()
        );
    }
}
