//! Integration tests: TLS impersonation against real Cloudflare-protected sites.
//!
//! Run: `cargo test --features tls-impersonation --test tls_cloudflare -- --ignored`
//!
//! All tests are `#[ignore]` because they require:
//!   - Network access
//!   - `tls-impersonation` feature flag
//!   - Real Cloudflare-protected endpoints (flaky by nature)

#[cfg(feature = "tls-impersonation")]
mod tls_cloudflare {
    use std::collections::HashMap;
    use std::time::Duration;

    use kaelo::types::{FetchRequest, Strategy};
    use kaelo::fetch::backend::FetchBackend;
    use kaelo::fetch::backends::TlsImpersonation;

    fn make_request(url: &str) -> FetchRequest {
        FetchRequest {
            url: url.to_string(),
            headers: HashMap::new(),
            timeout: Duration::from_secs(30),
            follow_redirects: true,
        }
    }

    #[test]
    #[ignore = "requires tls-impersonation feature"]
    fn tls_impersonation_new_succeeds() {
        let backend = TlsImpersonation::new();
        assert!(
            backend.is_ok(),
            "TlsImpersonation::new() should succeed: {:?}",
            backend.err()
        );
    }

    #[test]
    #[ignore = "requires tls-impersonation feature"]
    fn tls_impersonation_name_is_tls_chrome() {
        let backend = TlsImpersonation::new().expect("should build");
        assert_eq!(backend.name(), Strategy::TlsChrome);
    }

    #[test]
    #[ignore = "requires tls-impersonation feature"]
    fn tls_impersonation_supports_probe() {
        let backend = TlsImpersonation::new().expect("should build");
        assert!(backend.supports_probe());
    }

    #[tokio::test]
    #[ignore = "requires network and tls-impersonation feature"]
    async fn tls_fetch_non_cloudflare_control() {
        let backend = TlsImpersonation::new().expect("should build");
        let result = backend.fetch(make_request("https://example.com")).await;

        assert!(
            result.is_ok(),
            "fetch example.com via TLS should succeed: {:?}",
            result.err()
        );
        let resp = result.expect("already checked");
        assert_eq!(resp.status, 200, "example.com should return 200");
        assert!(!resp.body.is_empty(), "body should not be empty");

        let body = String::from_utf8_lossy(&resp.body);
        assert!(
            body.contains("Example Domain"),
            "should contain real page content, got: {}",
            &body[..body.len().min(200)]
        );
    }

    /// Fetch nowsecure.nl — a known Cloudflare-protected site.
    ///
    /// Pass criteria:
    ///   - Response status is 200 (not 403/challenge redirect)
    ///   - Body does NOT contain Cloudflare challenge text
    ///   - Body contains real page content
    ///
    /// If this test fails, TLS impersonation is NOT bypassing Cloudflare.
    /// Likely fixes: update wreq emulation profile to a newer Chrome version,
    /// or add headers (Accept-Language, etc.) to match real Chrome.
    #[tokio::test]
    #[ignore = "requires network and tls-impersonation feature"]
    async fn tls_cloudflare_nowsecure() {
        let backend = TlsImpersonation::new().expect("should build");
        let result = backend.fetch(make_request("https://nowsecure.nl/")).await;

        assert!(
            result.is_ok(),
            "fetch nowsecure.nl should succeed: {:?}",
            result.err()
        );
        let resp = result.expect("already checked");
        assert_eq!(
            resp.status, 200,
            "expected 200 (not a Cloudflare challenge), got: {}",
            resp.status
        );

        let body = String::from_utf8_lossy(&resp.body);

        assert!(
            !body.contains("Checking your browser"),
            "Got Cloudflare challenge page — TLS impersonation is NOT bypassing Cloudflare"
        );
        assert!(
            !body.contains("cf-browser-verification"),
            "Got Cloudflare browser verification — TLS fingerprint not accepted"
        );
        assert!(
            !body.contains("Just a moment"),
            "Got Cloudflare 'Just a moment' interstitial — impersonation failed"
        );

        assert!(
            body.contains("<!doctype") || body.contains("<!DOCTYPE") || body.contains("<html"),
            "response should contain real HTML content, got: {}",
            &body[..body.len().min(300)]
        );
    }

    /// Fetch tls.peet.ws/api/all — returns TLS fingerprint info as JSON.
    ///
    /// Pass criteria:
    ///   - Response is valid JSON (starts with `{`)
    ///   - Contains TLS fingerprint fields (ja3, ja4, or tls_version)
    ///
    /// If the fingerprint does NOT look like Chrome, the TLS impersonation
    /// is not working correctly. Check wreq_util::Emulation version.
    #[tokio::test]
    #[ignore = "requires network and tls-impersonation feature"]
    async fn tls_fingerprint_peet_ws() {
        let backend = TlsImpersonation::new().expect("should build");
        let result = backend
            .fetch(make_request("https://tls.peet.ws/api/all"))
            .await;

        assert!(
            result.is_ok(),
            "fetch tls.peet.ws should succeed: {:?}",
            result.err()
        );
        let resp = result.expect("already checked");
        assert_eq!(resp.status, 200, "expected 200, got: {}", resp.status);

        let body = String::from_utf8_lossy(&resp.body);

        assert!(
            body.trim_start().starts_with('{'),
            "response should be JSON, got: {}",
            &body[..body.len().min(200)]
        );

        assert!(
            body.contains("ja3") || body.contains("ja4") || body.contains("tls_version"),
            "response should contain TLS fingerprint fields (ja3/ja4/tls_version), got: {}",
            &body[..body.len().min(500)]
        );
    }

    #[tokio::test]
    #[ignore = "requires network and tls-impersonation feature"]
    async fn tls_cloudflare_utsavjewellers() {
        let backend = TlsImpersonation::new().expect("should build");
        let result = backend
            .fetch(make_request("https://utsavjewellers.com/"))
            .await;

        assert!(
            result.is_ok(),
            "fetch utsavjewellers.com should succeed: {:?}",
            result.err()
        );
        let resp = result.expect("already checked");

        let body = String::from_utf8_lossy(&resp.body);

        assert!(
            !body.contains("Checking your browser"),
            "Got Cloudflare challenge page for utsavjewellers.com — impersonation failed"
        );
        assert!(
            !body.contains("Just a moment"),
            "Got Cloudflare interstitial for utsavjewellers.com"
        );
    }
}
