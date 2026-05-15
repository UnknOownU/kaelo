use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::types::{FetchError, FetchRequest, FetchResponse};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeResult {
    Accessible,
    Blocked { status: u16 },
    Redirect { location: String, status: u16 },
    Error { message: String },
}

/// Lightweight probe request backend.
///
/// Defined in `kaelo-core` so the router can use it without depending on
/// `kaelo-fetch`. Concrete backends in `kaelo-fetch` implement this trait.
pub trait Prober: Send + Sync {
    fn probe(
        &self,
        request: FetchRequest,
    ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, FetchError>> + Send + '_>>;
}

pub async fn probe_domain(prober: &dyn Prober, url: &str) -> ProbeResult {
    let mut headers = HashMap::new();
    headers.insert("Range".to_string(), "bytes=0-0".to_string());

    let request = FetchRequest {
        url: url.to_string(),
        headers,
        timeout: Duration::from_secs(10),
        follow_redirects: false,
    };

    match prober.probe(request).await {
        Ok(response) => classify_response(response),
        Err(e) => classify_error(e),
    }
}

fn classify_response(response: FetchResponse) -> ProbeResult {
    match response.status {
        200 | 206 => ProbeResult::Accessible,

        301 | 302 | 307 | 308 => {
            let location = response
                .headers
                .get("location")
                .cloned()
                .unwrap_or_default();
            ProbeResult::Redirect {
                location,
                status: response.status,
            }
        }

        403 | 503 => ProbeResult::Blocked {
            status: response.status,
        },

        // 404 etc. — server responded, domain is reachable
        _ => ProbeResult::Accessible,
    }
}

fn classify_error(e: FetchError) -> ProbeResult {
    match e {
        FetchError::Timeout => ProbeResult::Error {
            message: "probe timed out".to_string(),
        },
        FetchError::HttpError(status) => {
            if status == 403 || status == 503 {
                ProbeResult::Blocked { status }
            } else {
                ProbeResult::Accessible
            }
        }
        FetchError::TlsError(msg)
        | FetchError::BrowserError(msg)
        | FetchError::NetworkError(msg) => ProbeResult::Error { message: msg },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16, headers: HashMap<String, String>) -> FetchResponse {
        FetchResponse {
            status,
            headers,
            body: Vec::new(),
            content_type: "text/html".to_string(),
            latency: Duration::from_millis(50),
        }
    }

    #[test]
    fn test_accessible_200() {
        assert_eq!(
            classify_response(response(200, HashMap::new())),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_accessible_206() {
        assert_eq!(
            classify_response(response(206, HashMap::new())),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_accessible_other_status() {
        assert_eq!(
            classify_response(response(404, HashMap::new())),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_blocked_403() {
        assert_eq!(
            classify_response(response(403, HashMap::new())),
            ProbeResult::Blocked { status: 403 }
        );
    }

    #[test]
    fn test_blocked_503() {
        assert_eq!(
            classify_response(response(503, HashMap::new())),
            ProbeResult::Blocked { status: 503 }
        );
    }

    #[test]
    fn test_redirect_301() {
        let mut h = HashMap::new();
        h.insert("location".to_string(), "https://example.com/".to_string());
        assert_eq!(
            classify_response(response(301, h)),
            ProbeResult::Redirect {
                location: "https://example.com/".to_string(),
                status: 301
            }
        );
    }

    #[test]
    fn test_redirect_302_no_location() {
        assert_eq!(
            classify_response(response(302, HashMap::new())),
            ProbeResult::Redirect {
                location: String::new(),
                status: 302
            }
        );
    }

    #[test]
    fn test_redirect_307() {
        let mut h = HashMap::new();
        h.insert(
            "location".to_string(),
            "https://new.example.com".to_string(),
        );
        assert_eq!(
            classify_response(response(307, h)),
            ProbeResult::Redirect {
                location: "https://new.example.com".to_string(),
                status: 307
            }
        );
    }

    #[test]
    fn test_redirect_308() {
        let mut h = HashMap::new();
        h.insert(
            "location".to_string(),
            "https://perm.example.com".to_string(),
        );
        assert_eq!(
            classify_response(response(308, h)),
            ProbeResult::Redirect {
                location: "https://perm.example.com".to_string(),
                status: 308
            }
        );
    }

    #[test]
    fn test_error_timeout() {
        assert_eq!(
            classify_error(FetchError::Timeout),
            ProbeResult::Error {
                message: "probe timed out".to_string()
            }
        );
    }

    #[test]
    fn test_error_http_403() {
        assert_eq!(
            classify_error(FetchError::HttpError(403)),
            ProbeResult::Blocked { status: 403 }
        );
    }

    #[test]
    fn test_error_http_503() {
        assert_eq!(
            classify_error(FetchError::HttpError(503)),
            ProbeResult::Blocked { status: 503 }
        );
    }

    #[test]
    fn test_error_http_other() {
        assert_eq!(
            classify_error(FetchError::HttpError(500)),
            ProbeResult::Accessible
        );
    }

    #[test]
    fn test_error_tls() {
        assert_eq!(
            classify_error(FetchError::TlsError("handshake failed".to_string())),
            ProbeResult::Error {
                message: "handshake failed".to_string()
            }
        );
    }

    #[test]
    fn test_error_network() {
        assert_eq!(
            classify_error(FetchError::NetworkError("connection refused".to_string())),
            ProbeResult::Error {
                message: "connection refused".to_string()
            }
        );
    }

    struct MockProber {
        response: Option<Result<FetchResponse, FetchError>>,
    }

    impl MockProber {
        fn with_ok(status: u16, headers: HashMap<String, String>) -> Self {
            Self {
                response: Some(Ok(response(status, headers))),
            }
        }

        fn with_err(e: FetchError) -> Self {
            Self {
                response: Some(Err(e)),
            }
        }
    }

    impl Prober for MockProber {
        fn probe(
            &self,
            _request: FetchRequest,
        ) -> Pin<Box<dyn Future<Output = Result<FetchResponse, FetchError>> + Send + '_>> {
            Box::pin(std::future::ready(self.response.clone().unwrap()))
        }
    }

    #[tokio::test]
    async fn test_probe_domain_accessible() {
        let prober = MockProber::with_ok(200, HashMap::new());
        let result = probe_domain(&prober, "https://example.com").await;
        assert_eq!(result, ProbeResult::Accessible);
    }

    #[tokio::test]
    async fn test_probe_domain_blocked() {
        let prober = MockProber::with_ok(403, HashMap::new());
        let result = probe_domain(&prober, "https://blocked.com").await;
        assert_eq!(result, ProbeResult::Blocked { status: 403 });
    }

    #[tokio::test]
    async fn test_probe_domain_redirect() {
        let mut h = HashMap::new();
        h.insert("location".to_string(), "https://other.com".to_string());
        let prober = MockProber::with_ok(301, h);
        let result = probe_domain(&prober, "https://redirect.com").await;
        assert_eq!(
            result,
            ProbeResult::Redirect {
                location: "https://other.com".to_string(),
                status: 301
            }
        );
    }

    #[tokio::test]
    async fn test_probe_domain_timeout() {
        let prober = MockProber::with_err(FetchError::Timeout);
        let result = probe_domain(&prober, "https://slow.com").await;
        assert_eq!(
            result,
            ProbeResult::Error {
                message: "probe timed out".to_string()
            }
        );
    }

    #[tokio::test]
    async fn test_probe_domain_network_error() {
        let prober = MockProber::with_err(FetchError::NetworkError("dns failed".to_string()));
        let result = probe_domain(&prober, "https://down.com").await;
        assert_eq!(
            result,
            ProbeResult::Error {
                message: "dns failed".to_string()
            }
        );
    }
}
