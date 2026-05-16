//! Shared browser pool for the headless Chromium backend.
//!
//! Lazily initializes a single Chrome instance and reuses it across concurrent
//! requests. If Chrome crashes, the next request automatically re-launches it.
//!
//! # Persistent sessions
//!
//! Call [`BrowserPool::get_or_create_session`] to obtain a page that persists
//! across multiple fetches. Pages within the same session share cookies and
//! browser state. Sessions expire after 5 minutes of inactivity and at most 3
//! concurrent sessions are allowed (oldest evicted on overflow).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ::chromiumoxide::browser::{Browser, BrowserConfig};
use ::chromiumoxide::Page;
use futures::StreamExt;
use kaelo_core::types::FetchError;
use tokio::sync::Mutex;

const SESSION_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Maximum number of concurrent sessions. Oldest is evicted on overflow.
const MAX_SESSIONS: usize = 3;

/// Pool of realistic Chrome User-Agent strings (no "HeadlessChrome").
const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
];

pub const WEBDRIVER_HIDE_JS: &str =
    "Object.defineProperty(navigator, 'webdriver', {get: () => undefined})";

/// Deterministic-ish index based on current time (no external rand crate).
fn pick_user_agent() -> &'static str {
    let idx = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0);
    USER_AGENTS[idx % USER_AGENTS.len()]
}

/// Randomize viewport around 1920×1080 using time-based seed.
pub fn random_viewport() -> (u32, u32) {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let width = 1800 + (seed % 121) as u32; // 1800..=1920
    let height = 900 + ((seed / 7) % 181) as u32; // 900..=1080
    (width, height)
}

struct BrowserSlot {
    browser: Arc<Browser>,
    _handler: tokio::task::JoinHandle<()>,
}

struct Session {
    page: Page,
    last_used: Instant,
}

/// Lazily initialized — the browser is launched on first use and reused across
/// concurrent requests. If Chrome crashes, the next request automatically re-launches it.
///
/// `BrowserPool` is cheaply [`Clone`]able — all clones share the same
/// underlying browser.
#[derive(Clone)]
pub struct BrowserPool {
    slot: Arc<Mutex<Option<BrowserSlot>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
}

impl Default for BrowserPool {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserPool {
    /// Create a new pool. **No browser is launched** until the first call to
    /// [`get_page`](Self::get_page).
    pub fn new() -> Self {
        Self {
            slot: Arc::new(Mutex::new(None)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Obtain a new [`Page`] navigated to `url`.
    ///
    /// 1. If a browser is already running, opens a new tab.
    /// 2. If the existing browser has crashed, clears it and re-launches.
    /// 3. If no browser exists yet, launches one (lazy init).
    pub async fn get_page(&self, url: &str) -> Result<Page, FetchError> {
        let browser = {
            let guard = self.slot.lock().await;
            guard.as_ref().map(|s| s.browser.clone())
        };

        if let Some(b) = browser {
            match b.new_page(url).await {
                Ok(page) => return Ok(page),
                Err(_) => {
                    // Browser likely crashed — clear the stale slot.
                    let mut guard = self.slot.lock().await;
                    if guard
                        .as_ref()
                        .map(|s| Arc::ptr_eq(&s.browser, &b))
                        .unwrap_or(false)
                    {
                        *guard = None;
                    }
                }
            }
        }

        // Double-check: another caller may have launched while we waited.
        let browser = {
            let mut guard = self.slot.lock().await;
            if let Some(slot) = guard.as_ref() {
                slot.browser.clone()
            } else {
                let (b, mut handler) = Self::launch_browser().await?;
                let hh = tokio::spawn(async move {
                    while let Some(event) = handler.next().await {
                        if event.is_err() {
                            break;
                        }
                    }
                });
                let arc = Arc::new(b);
                *guard = Some(BrowserSlot {
                    browser: arc.clone(),
                    _handler: hh,
                });
                arc
            }
        };

        browser
            .new_page(url)
            .await
            .map_err(|e| FetchError::BrowserError(format!("failed to open page: {e}")))
    }

    /// Obtain a [`Page`] for a persistent session.
    ///
    /// If a session with `session_id` already exists and has not expired, its
    /// page is navigated to `url` and returned. Otherwise a fresh page is
    /// created, stored in the session map, and returned.
    ///
    /// Expired sessions (> 5 min idle) are evicted on every call. If adding a
    /// new session would exceed [`MAX_SESSIONS`], the least-recently-used
    /// session is evicted first.
    pub async fn get_or_create_session(
        &self,
        session_id: &str,
        url: &str,
    ) -> Result<Page, FetchError> {
        let mut sessions = self.sessions.lock().await;

        let expired_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.last_used.elapsed() > SESSION_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired_ids {
            if let Some(session) = sessions.remove(&id) {
                let _ = session.page.close().await;
            }
        }

        if let Some(session) = sessions.get_mut(session_id) {
            session.last_used = Instant::now();
            let page = session.page.clone();
            drop(sessions);
            page.goto(url)
                .await
                .map_err(|e| FetchError::BrowserError(format!("navigation failed: {e}")))?;
            return Ok(page);
        }

        while sessions.len() >= MAX_SESSIONS {
            let oldest = sessions
                .iter()
                .min_by_key(|(_, s)| s.last_used)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => {
                    if let Some(old) = sessions.remove(&id) {
                        let _ = old.page.close().await;
                    }
                }
                None => break,
            }
        }

        drop(sessions);

        let page = self.get_page(url).await?;

        let mut sessions = self.sessions.lock().await;
        sessions.insert(
            session_id.to_string(),
            Session {
                page: page.clone(),
                last_used: Instant::now(),
            },
        );

        Ok(page)
    }

    pub async fn close_session(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.remove(session_id) {
            let _ = session.page.close().await;
        }
    }

    pub async fn shutdown(&self) {
        let mut guard = self.slot.lock().await;
        *guard = None;

        let mut sessions = self.sessions.lock().await;
        for (_, session) in sessions.drain() {
            let _ = session.page.close().await;
        }
    }

    async fn launch_browser() -> Result<(Browser, chromiumoxide::Handler), FetchError> {
        let ua = pick_user_agent();
        let config = BrowserConfig::builder()
            .arg("--disable-blink-features=AutomationControlled")
            .arg("--disable-infobars")
            .arg("--window-size=1920,1080")
            .arg(format!("--user-agent={ua}"))
            .build()
            .map_err(|e| {
                FetchError::BrowserError(format!("failed to build browser config: {e}"))
            })?;

        Browser::launch(config).await.map_err(|e| {
            FetchError::BrowserError(format!(
                "failed to launch Chrome — is Chromium installed? {e}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_launch_browser() {
        let pool = BrowserPool::new();
        assert!(
            pool.slot.try_lock().unwrap().is_none(),
            "pool should start empty (lazy init)"
        );
    }

    #[test]
    fn clone_shares_same_inner() {
        let pool = BrowserPool::new();
        let clone = pool.clone();
        assert!(
            Arc::ptr_eq(&pool.slot, &clone.slot),
            "clones must share the same inner Arc"
        );
    }

    #[tokio::test]
    async fn shutdown_clears_slot() {
        let pool = BrowserPool::new();
        pool.shutdown().await;
        assert!(
            pool.slot.try_lock().unwrap().is_none(),
            "shutdown should clear the slot"
        );
    }

    #[test]
    fn new_pool_has_empty_sessions() {
        let pool = BrowserPool::new();
        assert!(
            pool.sessions.try_lock().unwrap().is_empty(),
            "sessions should start empty"
        );
    }

    #[test]
    fn clone_shares_sessions() {
        let pool = BrowserPool::new();
        let clone = pool.clone();
        assert!(
            Arc::ptr_eq(&pool.sessions, &clone.sessions),
            "clones must share the same sessions Arc"
        );
    }

    #[tokio::test]
    async fn close_session_is_noop_when_absent() {
        let pool = BrowserPool::new();
        pool.close_session("nonexistent").await;
        assert!(pool.sessions.try_lock().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn session_creates_and_reuses_page() {
        let pool = BrowserPool::new();

        let p1 = pool
            .get_or_create_session("s1", "about:blank")
            .await
            .expect("first session call");
        let p2 = pool
            .get_or_create_session("s1", "about:blank")
            .await
            .expect("second session call");

        assert_eq!(p1.target_id(), p2.target_id());

        pool.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn max_three_sessions_enforced() {
        let pool = BrowserPool::new();

        for i in 0..4 {
            let sid = format!("s{i}");
            pool.get_or_create_session(&sid, "about:blank")
                .await
                .expect("session creation");
        }

        let sessions = pool.sessions.try_lock().unwrap();
        assert_eq!(sessions.len(), MAX_SESSIONS);
        assert!(
            !sessions.contains_key("s0"),
            "oldest session should have been evicted"
        );

        drop(sessions);
        pool.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn expired_session_is_evicted() {
        let pool = BrowserPool::new();

        pool.get_or_create_session("old", "about:blank")
            .await
            .expect("create old session");

        {
            let mut sessions = pool.sessions.lock().await;
            if let Some(s) = sessions.get_mut("old") {
                s.last_used = Instant::now() - SESSION_TIMEOUT - Duration::from_secs(1);
            }
        }

        pool.get_or_create_session("old", "about:blank")
            .await
            .expect("recreate expired session");

        let sessions = pool.sessions.try_lock().unwrap();
        assert!(sessions.contains_key("old"));
        assert!(
            sessions["old"].last_used.elapsed() < Duration::from_secs(5),
            "recreated session should have fresh timestamp"
        );

        drop(sessions);
        pool.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn no_session_id_is_stateless() {
        let pool = BrowserPool::new();

        let p1 = pool.get_page("about:blank").await;
        let p2 = pool.get_page("about:blank").await;
        assert!(p1.is_ok());
        assert!(p2.is_ok());
        assert!(
            pool.sessions.try_lock().unwrap().is_empty(),
            "stateless get_page should not create sessions"
        );

        pool.shutdown().await;
    }

    #[test]
    fn user_agents_contain_no_headless() {
        for ua in USER_AGENTS {
            assert!(
                !ua.contains("Headless"),
                "UA must not contain 'Headless': {ua}"
            );
            assert!(ua.contains("Chrome/"), "UA should look like Chrome: {ua}");
        }
    }

    #[test]
    fn webdriver_hide_js_is_valid_expression() {
        assert!(WEBDRIVER_HIDE_JS.contains("navigator"));
        assert!(WEBDRIVER_HIDE_JS.contains("webdriver"));
        assert!(WEBDRIVER_HIDE_JS.contains("undefined"));
        assert!(!WEBDRIVER_HIDE_JS.contains("Headless"));
    }

    #[test]
    fn random_viewport_in_expected_range() {
        let (w, h) = random_viewport();
        assert!((1800..=1920).contains(&w), "width {w} out of range");
        assert!((900..=1080).contains(&h), "height {h} out of range");
    }

    #[test]
    fn pick_user_agent_returns_valid_ua() {
        let ua = pick_user_agent();
        assert!(USER_AGENTS.contains(&ua));
        assert!(!ua.contains("Headless"));
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn get_page_launches_browser_lazily() {
        let pool = BrowserPool::new();
        assert!(pool.slot.try_lock().unwrap().is_none());

        let page = pool.get_page("about:blank").await;
        assert!(page.is_ok(), "get_page failed: {:?}", page.err());

        assert!(
            pool.slot.try_lock().unwrap().is_some(),
            "slot should contain a browser after get_page"
        );

        pool.shutdown().await;
    }

    #[tokio::test]
    #[ignore = "requires Chromium installed"]
    async fn get_page_reuses_browser() {
        let pool = BrowserPool::new();

        let p1 = pool.get_page("about:blank").await;
        let p2 = pool.get_page("about:blank").await;
        assert!(p1.is_ok());
        assert!(p2.is_ok());

        let guard = pool.slot.try_lock().unwrap();
        assert!(guard.is_some());

        pool.shutdown().await;
    }
}
