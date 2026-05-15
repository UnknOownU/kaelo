use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use kaelo_core::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use crate::backend::FetchBackend;

pub struct HeadlessBrowser {
    browser: Arc<Browser>,
    _handler: Arc<tokio::task::JoinHandle<()>>,
}

impl HeadlessBrowser {
    pub async fn new() -> Result<Self, FetchError> {
        let config = BrowserConfig::builder()
            .build()
            .map_err(|e| FetchError::BrowserError(format!("failed to build browser config: {e}")))?;

        let (browser, mut handler) = Browser::launch(config).await.map_err(|e| {
            FetchError::BrowserError(format!(
                "failed to launch Chrome — is Chromium installed? {e}"
            ))
        })?;

        let handler_handle = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            browser: Arc::new(browser),
            _handler: Arc::new(handler_handle),
        })
    }
}

impl FetchBackend for HeadlessBrowser {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let start = Instant::now();
        let timeout = request.timeout;

        let result = tokio::time::timeout(timeout, async {
            let page = self
                .browser
                .new_page(&request.url)
                .await
                .map_err(|e| FetchError::BrowserError(format!("failed to open page: {e}")))?;

            page.wait_for_navigation()
                .await
                .map_err(|e| FetchError::BrowserError(format!("navigation failed: {e}")))?;

            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let _ = page
                .evaluate(r#"document.querySelector('[aria-label="Accept"], .accept-cookies, #accept-cookies, button[mode="primary"]')?.click()"#)
                .await;

            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            let html = page
                .content()
                .await
                .map_err(|e| FetchError::BrowserError(format!("failed to get page content: {e}")))?;

            page.close()
                .await
                .map_err(|e| FetchError::BrowserError(format!("failed to close page: {e}")))?;

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

    fn name(&self) -> Strategy {
        Strategy::Headless
    }

    fn supports_probe(&self) -> bool {
        false
    }
}

// TODO: Browser pooling — currently launches Chrome per request (2-3s overhead).
// Add a shared BrowserPool that reuses a single browser instance across requests.

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn test_headless_new() {
        let browser = HeadlessBrowser::new().await;
        assert!(browser.is_ok(), "failed to launch headless browser");
    }

    #[test]
    fn test_headless_name_returns_correct_strategy() {
        assert_eq!(Strategy::Headless, Strategy::Headless);
    }
}
