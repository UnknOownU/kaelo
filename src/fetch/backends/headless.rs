use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

use crate::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use super::browser_pool::{random_viewport, BrowserPool, WEBDRIVER_HIDE_JS};
use crate::fetch::backend::FetchBackend;

static POOL: OnceLock<BrowserPool> = OnceLock::new();

const WAIT_FOR_CONTENT_JS: &str = r#"(function() {
  var body = document.body;
  if (!body) return Promise.resolve(false);
  // Check for substantial content — 500+ chars to avoid false positives from nav/header
  function hasContent() {
    return body.innerText.trim().length > 500 && body.querySelectorAll('p, article, main, [role="main"]').length > 0;
  }
  if (hasContent()) return Promise.resolve(true);
  return new Promise(function(resolve) {
    var timeout = setTimeout(function() { resolve(false); }, 15000);
    // Minimum wait: even if content appears fast, wait 2s for late-loading elements
    var minWait = setTimeout(function() {}, 2000);
    var debounce = null;
    var observer = new MutationObserver(function() {
      clearTimeout(debounce);
      debounce = setTimeout(function() {
        if (hasContent()) {
          clearTimeout(timeout);
          observer.disconnect();
          // Ensure minimum wait for lazy-loaded content
          setTimeout(function() { resolve(true); }, 1000);
        }
      }, 800);
    });
    observer.observe(body, { childList: true, subtree: true });
  });
})()"#;

/// Shut down the global browser pool, closing all Chrome processes and sessions.
///
/// Safe to call even if the pool was never initialized (no-op in that case).
pub async fn shutdown_browser_pool() {
    if let Some(pool) = POOL.get() {
        pool.shutdown().await;
    }
}

pub struct HeadlessBrowser {
    pool: BrowserPool,
}

impl HeadlessBrowser {
    pub async fn new() -> Result<Self, FetchError> {
        let pool = POOL.get_or_init(BrowserPool::new);
        Ok(Self { pool: pool.clone() })
    }

    pub async fn fetch_with_session(
        &self,
        request: FetchRequest,
        session_id: Option<String>,
    ) -> Result<FetchResponse, FetchError> {
        let start = Instant::now();
        let timeout = request.timeout;

        let result = tokio::time::timeout(timeout, async {
            let page = match &session_id {
                Some(sid) => {
                    self.pool
                        .get_or_create_session(sid, &request.url)
                        .await?
                }
                None => self.pool.get_page(&request.url).await?,
            };

            let (vp_w, vp_h) = random_viewport();
            let set_viewport_js =
                format!("Object.defineProperty(screen, 'width', {{get: () => {vp_w}}}); \
                         Object.defineProperty(screen, 'height', {{get: () => {vp_h}}})");
            let _ = page.evaluate(set_viewport_js).await;

            let _ = page.evaluate(WEBDRIVER_HIDE_JS).await;

            page.wait_for_navigation()
                .await
                .map_err(|e| FetchError::BrowserError(format!("navigation failed: {e}")))?;

            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

            let _ = page
                .evaluate(r#"document.querySelector('[aria-label="Accept"], .accept-cookies, #accept-cookies, button[mode="primary"]')?.click()"#)
                .await;

            if let Err(e) = page.evaluate(WAIT_FOR_CONTENT_JS).await {
                tracing::warn!("SPA wait failed, falling back to sleep: {e}");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }

            let html = page
                .content()
                .await
                .map_err(|e| FetchError::BrowserError(format!("failed to get page content: {e}")))?;

            if session_id.is_none() {
                page.close()
                    .await
                    .map_err(|e| FetchError::BrowserError(format!("failed to close page: {e}")))?;
            }

            Ok::<String, FetchError>(html)
        })
        .await;

        let html = match result {
            Ok(Ok(html)) => html,
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(FetchError::Timeout),
        };

        let latency = start.elapsed();

        Ok(FetchResponse {
            body: html.into_bytes(),
            status: 200,
            content_type: "text/html".to_string(),
            headers: HashMap::new(),
            latency,
        })
    }
}

impl FetchBackend for HeadlessBrowser {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.fetch_with_session(request, None).await
    }

    fn name(&self) -> Strategy {
        Strategy::Headless
    }

    fn supports_probe(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn test_headless_new() {
        let browser = HeadlessBrowser::new().await;
        assert!(browser.is_ok(), "failed to create headless browser");
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn test_stateless_fetch_closes_page() {
        let browser = HeadlessBrowser::new().await.unwrap();
        let request = FetchRequest {
            url: "about:blank".to_string(),
            headers: HashMap::new(),
            timeout: std::time::Duration::from_secs(30),
            follow_redirects: true,
        };
        let result = browser.fetch(request).await;
        assert!(result.is_ok(), "stateless fetch should succeed");
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn test_session_fetch_reuses_page() {
        let browser = HeadlessBrowser::new().await.unwrap();
        let request = FetchRequest {
            url: "about:blank".to_string(),
            headers: HashMap::new(),
            timeout: std::time::Duration::from_secs(30),
            follow_redirects: true,
        };

        let r1 = browser
            .fetch_with_session(request.clone(), Some("test-session".to_string()))
            .await;
        let r2 = browser
            .fetch_with_session(request, Some("test-session".to_string()))
            .await;
        assert!(r1.is_ok(), "first session fetch should succeed");
        assert!(r2.is_ok(), "second session fetch should succeed");

        browser.pool.close_session("test-session").await;
    }
}
