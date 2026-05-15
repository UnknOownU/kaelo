//! Spike: Validate wreq crate (=6.0.0-rc.28) for TLS impersonation (Chrome fingerprint)
//!
//! Run: `cargo test -p kaelo-fetch --features tls-impersonation spike_wreq -- --ignored`
//!
//! PURPOSE: Throwaway spike to answer:
//!   1. Does wreq build with tls-impersonation feature?
//!   2. Can we create a client with Chrome TLS fingerprint?
//!   3. Can we fetch a real URL using wreq impersonation?
//!   4. How to wrap wreq behind FetchBackend trait? (documented below)

// ── Adapter Pattern for FetchBackend (Phase 1 / T12) ──────────────────────
//
// The FetchBackend trait should use Kaelo-owned types, NOT wreq types directly.
// Adapter approach:
//
//   pub trait FetchBackend {
//       async fn fetch(&self, url: &str) -> Result<FetchResponse>;
//   }
//
//   pub struct FetchResponse {
//       pub status: u16,
//       pub body: String,
//       pub headers: HashMap<String, String>,
//   }
//
//   pub struct WreqBackend { client: wreq::Client }
//
//   impl FetchBackend for WreqBackend {
//       async fn fetch(&self, url: &str) -> Result<FetchResponse> {
//           let resp = self.client.get(url).send().await?;
//           Ok(FetchResponse {
//               status: resp.status().as_u16(),
//               body: resp.text().await?,
//               headers: resp.headers().iter()
//                   .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
//                   .collect(),
//           })
//       }
//   }
//
// Construction:
//   let client = wreq::Client::builder()
//       .emulation(wreq_util::Emulation::Chrome131)
//       .build()?;
//   let backend = WreqBackend { client };
//
// Key differences from reqwest:
//   - Import: `wreq::Client` + `wreq_util::Emulation` (two crates)
//   - Method: `.emulation()` not `.impersonate()`
//   - Response API: same as reqwest (`.text()`, `.status()`, `.headers()`)
//   - Error type: `wreq::Error` (not reqwest::Error)

#[cfg(feature = "tls-impersonation")]
mod wreq_spike {
    use wreq::Client;
    use wreq_util::Emulation;

    /// Verify wreq compiles with Chrome emulation profile.
    #[test]
    #[ignore = "requires network"]
    fn spike_build_client_with_chrome_emulation() {
        let client = Client::builder().emulation(Emulation::Chrome131).build();

        assert!(
            client.is_ok(),
            "Failed to build wreq client: {:?}",
            client.err()
        );
        let _client = client.unwrap();
    }

    /// Verify wreq can fetch a real URL with Chrome TLS fingerprint.
    #[tokio::test]
    #[ignore = "requires network"]
    async fn spike_fetch_with_chrome_fingerprint() {
        let client = Client::builder()
            .emulation(Emulation::Chrome131)
            .build()
            .expect("wreq client should build");

        let resp = client
            .get("https://tls.peet.ws/api/all")
            .send()
            .await
            .expect("request should succeed");

        let status = resp.status();
        assert!(status.is_success(), "Expected 2xx, got: {}", status);

        let body = resp.text().await.expect("should read body");
        assert!(!body.is_empty(), "response body should not be empty");

        // The peet.ws API returns TLS fingerprint info in JSON
        // Should show Chrome-like JA3/JA4 fingerprint
        println!("TLS fingerprint response:\n{}", body);
    }

    /// Verify wreq response API mirrors reqwest (status, text, headers).
    #[tokio::test]
    #[ignore = "requires network"]
    async fn spike_response_api_compat() {
        let client = Client::builder()
            .emulation(Emulation::Chrome131)
            .build()
            .expect("wreq client should build");

        let resp = client
            .get("https://httpbin.org/get")
            .send()
            .await
            .expect("request should succeed");

        // Status API (same as reqwest)
        let status = resp.status();
        assert_eq!(status.as_u16(), 200u16);

        // Headers API (same as reqwest)
        let headers = resp.headers();
        assert!(!headers.is_empty(), "should have response headers");
        let content_type = headers
            .get("content-type")
            .expect("should have content-type")
            .to_str()
            .expect("content-type should be valid str");
        assert!(
            content_type.contains("application/json"),
            "got: {}",
            content_type
        );

        // Body text API (same as reqwest)
        let body = resp.text().await.expect("should read body as text");
        assert!(
            body.contains("\"url\""),
            "httpbin response should contain url field"
        );

        println!("httpbin response:\n{}", body);
    }

    /// Enumerate available Chrome versions in wreq_util::Emulation.
    /// Documents which Chrome profiles are available.
    #[test]
    fn spike_available_chrome_versions() {
        // Build clients with different Chrome versions to verify they all compile
        let versions: Vec<(&str, Emulation)> = vec![
            ("Chrome100", Emulation::Chrome100),
            ("Chrome107", Emulation::Chrome107),
            ("Chrome120", Emulation::Chrome120),
            ("Chrome131", Emulation::Chrome131),
            ("Chrome133", Emulation::Chrome133),
        ];

        for (name, emulation) in versions {
            let result = Client::builder().emulation(emulation).build();
            assert!(result.is_ok(), "Failed to build client for {}", name);
            println!("{}: OK", name);
        }
    }
}
