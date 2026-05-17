use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use kaelo_core::extraction;
use kaelo_core::extraction::quality::is_content_valuable;
use kaelo_core::extraction::spa_detect;
use kaelo_core::router::fallback::{classify_error, next_strategy, ErrorClass};
use kaelo_core::search::ddg::DuckDuckGoBackend;
use kaelo_core::search::searxng::SearXngBackend;
use kaelo_core::search::{SearchBackend, SearchResult};
use kaelo_core::storage::content_cache::ContentCache;
use kaelo_core::storage::route_cache::{FetchResult, RouteCache};
use kaelo_core::storage::Storage;
use kaelo_core::types::{FetchError, FetchRequest, FetchResponse, Strategy};
use kaelo_core::update;
use kaelo_fetch::backend::FetchBackend;
use kaelo_fetch::backends::HttpSimple;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_handler, tool_router, transport::stdio, ErrorData, ServiceExt};

struct KaeloState {
    storage: Mutex<Storage>,
    searxng_url: Option<String>,
    search_backend: Option<String>,
    pending_update: Mutex<Option<String>>,
}

impl fmt::Debug for KaeloState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KaeloState").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct KaeloServer {
    state: Arc<KaeloState>,
}

impl KaeloServer {
    pub fn new(storage: Storage) -> Self {
        Self::with_config(storage, None, None)
    }

    pub fn with_config(
        storage: Storage,
        searxng_url: Option<String>,
        search_backend: Option<String>,
    ) -> Self {
        let state = Arc::new(KaeloState {
            storage: Mutex::new(storage),
            searxng_url,
            search_backend,
            pending_update: Mutex::new(None),
        });

        // Spawn background update check (fire-and-forget)
        let update_state = state.clone();
        let handle = tokio::runtime::Handle::try_current();
        if let Ok(handle) = handle {
            handle.spawn(async move {
                match update::check_for_update().await {
                    Ok(Some(info)) => {
                        let msg = format!(
                            "\n\n[Kaelo: v{} → v{} available — restart to update. {}]",
                            info.current, info.latest, info.release_url
                        );
                        if let Ok(mut guard) = update_state.pending_update.lock() {
                            *guard = Some(msg);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::debug!("Background update check failed: {e}");
                    }
                }
            });
        }

        Self { state }
    }

    fn get_update_notice(&self) -> Option<String> {
        self.state
            .pending_update
            .lock()
            .ok()
            .and_then(|g| g.clone())
    }

    fn success_with_notice(&self, text: String) -> CallToolResult {
        let mut text = text;
        if let Some(notice) = self.get_update_notice() {
            text.push_str(&notice);
        }
        CallToolResult::success(vec![Content::text(text)])
    }
}

impl Default for KaeloServer {
    fn default() -> Self {
        let storage = Storage::open(":memory:").expect("in-memory storage should always succeed");
        Self::new(storage)
    }
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WebFetchParams {
    /// The URL to fetch
    pub url: String,
    /// Fetch strategy. LEAVE EMPTY to let Kaelo auto-detect the best strategy
    /// (recommended). Only set if you have a specific reason to force a backend.
    /// Options: "HttpSimple", "TlsChrome", "TlsMobile", "Headless", "PublicApi"
    #[serde(default)]
    pub strategy: Option<String>,
    /// Maximum token budget (approximate character count / 4). 0 = no limit.
    #[serde(default)]
    pub token_budget: Option<u32>,
    /// CSS selector or section to focus on (extract only matching content)
    #[serde(default)]
    pub focus: Option<String>,
    /// Skip cache and force fresh fetch
    #[serde(default)]
    pub no_cache: Option<bool>,
    /// Browser session ID — pages in the same session share cookies/state.
    /// Sessions expire after 5 min of inactivity. Max 3 concurrent sessions.
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WebSearchParams {
    /// Search query string
    pub query: String,
    /// Maximum number of results (default 5)
    #[serde(default)]
    pub max_results: Option<u32>,
    /// When true, fetch the top result's full content (search -> fetch pipeline)
    #[serde(default)]
    pub fetch_content: Option<bool>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FetchUrlsParams {
    /// List of URLs to fetch (max 10)
    pub urls: Vec<String>,
    /// Fetch strategy for all URLs: "HttpSimple", "TlsChrome", "TlsMobile", "Headless", "PublicApi"
    #[serde(default)]
    pub strategy: Option<String>,
    /// Maximum token budget per URL (approximate character count / 4). 0 = no limit.
    #[serde(default)]
    pub token_budget: Option<u32>,
}

fn internal_err(msg: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(msg.to_string(), None)
}

#[allow(unexpected_cfgs)]
async fn fetch_with_strategy(
    strategy: &Strategy,
    request: FetchRequest,
    session_id: Option<String>,
) -> Result<FetchResponse, FetchError> {
    match strategy {
        Strategy::HttpSimple => HttpSimple::new()?.fetch(request).await,
        Strategy::TlsChrome | Strategy::TlsMobile => {
            #[cfg(feature = "tls-impersonation")]
            {
                use kaelo_fetch::backends::TlsImpersonation;
                TlsImpersonation::new()?.fetch(request).await
            }
            #[cfg(not(feature = "tls-impersonation"))]
            {
                HttpSimple::new()?.fetch(request).await
            }
        }
        Strategy::Headless => {
            use kaelo_fetch::backends::HeadlessBrowser;
            let browser = HeadlessBrowser::new().await?;
            browser.fetch_with_session(request, session_id).await
        }
        Strategy::PublicApi => HttpSimple::new()?.fetch(request).await,
    }
}

struct ResolvedStrategy {
    strategy: Strategy,
}

fn resolve_strategy(storage: &Storage, url: &str) -> ResolvedStrategy {
    let domain = extract_domain(url).unwrap_or_default();
    let route_cache = RouteCache::new(storage);
    let strategy = route_cache
        .get_best_strategy(&domain)
        .ok()
        .flatten()
        .and_then(|cs| parse_strategy_name(&cs.strategy).ok())
        .unwrap_or(Strategy::HttpSimple);
    ResolvedStrategy { strategy }
}

fn check_content_cache(
    storage: &Storage,
    url: &str,
    strategy: Option<&Strategy>,
) -> Option<String> {
    let cache = ContentCache::new(storage);
    match strategy {
        Some(s) => cache.get(url, s).ok().flatten().map(|c| c.content),
        None => cache.get_any(url).ok().flatten().map(|c| c.content),
    }
}

#[allow(clippy::too_many_arguments)]
fn store_and_record(
    storage: &Storage,
    url: &str,
    domain: &str,
    strategy: &Strategy,
    markdown: &str,
    content_type: &str,
    latency_ms: u64,
    status: u16,
    is_spa: bool,
) {
    let cache = ContentCache::new(storage);
    let _ = cache.put(
        url,
        strategy,
        markdown,
        content_type,
        Duration::from_secs(3600),
    );

    // Prevent cache poisoning: if the response is from a SPA domain but the
    // strategy is NOT Headless, do not record this strategy as "successful"
    // for the domain — it likely returned an empty JS shell, not real content.
    if is_spa && *strategy != Strategy::Headless {
        tracing::debug!(
            domain,
            strategy = ?strategy,
            "Skipping route cache recording: SPA content with non-Headless strategy"
        );
        return;
    }

    let route_cache = RouteCache::new(storage);
    let _ = route_cache.upsert_strategy(
        domain,
        &format!("{strategy:?}"),
        &FetchResult {
            latency_ms,
            success: status < 400,
            headers: None,
        },
    );
}

fn extract_domain(url: &str) -> Result<String> {
    let no_proto = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| anyhow::anyhow!("invalid URL: no protocol"))?;
    let domain = no_proto.split('/').next().unwrap_or(no_proto);
    let domain = domain.split(':').next().unwrap_or(domain);
    Ok(domain.to_string())
}

fn parse_strategy_name(name: &str) -> Result<Strategy> {
    match name {
        "HttpSimple" => Ok(Strategy::HttpSimple),
        "TlsChrome" => Ok(Strategy::TlsChrome),
        "TlsMobile" => Ok(Strategy::TlsMobile),
        "Headless" => Ok(Strategy::Headless),
        "PublicApi" => Ok(Strategy::PublicApi),
        other => Err(anyhow::anyhow!("unknown strategy: {other}")),
    }
}

#[tool_router]
impl KaeloServer {
    #[tool(
        name = "web_fetch",
        description = "Fetch a URL and return clean Markdown content"
    )]
    async fn web_fetch(
        &self,
        Parameters(params): Parameters<WebFetchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let no_cache = params.no_cache.unwrap_or(false);

        let explicit_strategy = params
            .strategy
            .as_deref()
            .map(|s| parse_strategy_name(s).unwrap_or(Strategy::HttpSimple));

        if no_cache {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            let cache = ContentCache::new(&storage);
            let _ = cache.invalidate(&params.url, explicit_strategy.as_ref());
        }

        if !no_cache {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            if let Some(content) =
                check_content_cache(&storage, &params.url, explicit_strategy.as_ref())
            {
                tracing::info!(url = %params.url, "Cache hit");
                let result = apply_token_budget(content, params.token_budget);
                return Ok(self.success_with_notice(result));
            }
        }

        let initial_strategy = match explicit_strategy {
            Some(s) => s,
            None => {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                resolve_strategy(&storage, &params.url).strategy
            }
        };

        let max_attempts = 3u32;
        let mut visited: HashSet<Strategy> = HashSet::new();
        let mut attempts = 0u32;
        let mut current_strategy = initial_strategy;
        let mut last_error: Option<String> = None;

        loop {
            if attempts >= max_attempts {
                break;
            }

            visited.insert(current_strategy.clone());
            attempts += 1;

            let request = FetchRequest {
                url: params.url.clone(),
                headers: HashMap::new(),
                timeout: Duration::from_secs(30),
                follow_redirects: true,
            };

            tracing::info!(
                url = %params.url,
                strategy = ?current_strategy,
                attempt = attempts,
                "Fetching"
            );

            let start = std::time::Instant::now();
            match fetch_with_strategy(&current_strategy, request, params.session.clone()).await {
                Ok(response) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    tracing::info!(
                        url = %params.url,
                        strategy = ?current_strategy,
                        latency_ms,
                        "Fetch completed"
                    );

                    let html = String::from_utf8_lossy(&response.body);
                    let extracted = extraction::extract(&html, &params.url, &response.content_type)
                        .map_err(|e| internal_err(format!("extraction error: {e}")))?;

                    let markdown = match extracted {
                        Some(e) if !e.text_content.trim().is_empty() => e.text_content,
                        _ => String::new(),
                    };

                    let is_spa = spa_detect::detect_spa(&html).is_some();

                    if is_content_valuable(&markdown) {
                        let domain =
                            extract_domain(&params.url).unwrap_or_else(|_| "unknown".to_string());
                        {
                            let storage = self.state.storage.lock().map_err(internal_err)?;
                            store_and_record(
                                &storage,
                                &params.url,
                                &domain,
                                &current_strategy,
                                &markdown,
                                &response.content_type,
                                latency_ms,
                                response.status,
                                is_spa,
                            );
                        }
                        let result = apply_token_budget(markdown, params.token_budget);
                        return Ok(self.success_with_notice(result));
                    } else {
                        tracing::warn!(
                            url = %params.url,
                            strategy = ?current_strategy,
                            "Content not valuable enough to cache, trying next strategy"
                        );

                        if is_spa && !visited.contains(&Strategy::Headless) {
                            tracing::info!(
                                url = %params.url,
                                spa = ?spa_detect::detect_spa(&html),
                                "SPA detected, skipping to Headless"
                            );
                            current_strategy = Strategy::Headless;
                            continue;
                        }

                        match next_strategy(
                            &current_strategy,
                            &visited,
                            max_attempts,
                            attempts,
                            &FetchError::NetworkError("low quality content".into()),
                        ) {
                            Some(next) => {
                                tracing::warn!(
                                    url = %params.url,
                                    from = ?current_strategy,
                                    to = ?next,
                                    "Falling back"
                                );
                                current_strategy = next;
                                continue;
                            }
                            None => {
                                let mut result = apply_token_budget(markdown, params.token_budget);
                                let warning = format!(
                                    "\n\n[Kaelo: unable to render JavaScript content for this URL. Tried: {} — none returned usable content.]",
                                    visited.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ")
                                );
                                result.push_str(&warning);
                                return Ok(self.success_with_notice(result));
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(
                        url = %params.url,
                        strategy = ?current_strategy,
                        error = %e,
                        "Fetch failed"
                    );
                    let class = classify_error(&e);

                    match class {
                        ErrorClass::NotFound | ErrorClass::RateLimited => {
                            return Err(internal_err(e));
                        }
                        ErrorClass::Transient | ErrorClass::Blocking => {
                            last_error = Some(e.to_string());
                            match next_strategy(
                                &current_strategy,
                                &visited,
                                max_attempts,
                                attempts,
                                &e,
                            ) {
                                Some(next) => {
                                    tracing::warn!(
                                        url = %params.url,
                                        from = ?current_strategy,
                                        to = ?next,
                                        "Falling back"
                                    );
                                    current_strategy = next;
                                    continue;
                                }
                                None => {
                                    return Err(internal_err(format!(
                                        "All fetch strategies exhausted for {}: {}",
                                        params.url,
                                        last_error.as_deref().unwrap_or("unknown")
                                    )));
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(internal_err(format!(
            "All fetch strategies exhausted for {}: {}",
            params.url,
            last_error.as_deref().unwrap_or("unknown")
        )))
    }

    #[tool(
        name = "fetch_urls",
        description = "Fetch multiple URLs in parallel and return combined Markdown content with --- separators"
    )]
    async fn fetch_urls(
        &self,
        Parameters(params): Parameters<FetchUrlsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if params.urls.is_empty() {
            return Err(internal_err("At least one URL required"));
        }
        if params.urls.len() > 10 {
            return Err(internal_err("Maximum 10 URLs per batch"));
        }

        tracing::info!(urls = ?params.urls, "Batch fetching {} URLs", params.urls.len());

        let state = self.state.clone();
        let strategy_name = params.strategy;
        let token_budget = params.token_budget;

        let mut handles: Vec<tokio::task::JoinHandle<String>> = Vec::new();

        for url in params.urls {
            let state = state.clone();
            let strategy_name = strategy_name.clone();

            handles.push(tokio::spawn(async move {
                let explicit_strategy = strategy_name.as_deref()
                    .map(|s| parse_strategy_name(s).unwrap_or(Strategy::HttpSimple));

                let (cached, initial_strategy) = {
                    let storage = match state.storage.lock() {
                        Ok(s) => s,
                        Err(e) => return format!("[error: {e}]"),
                    };
                    let cached = check_content_cache(&storage, &url, explicit_strategy.as_ref());
                    let strategy = match explicit_strategy {
                        Some(s) => s,
                        None => resolve_strategy(&storage, &url).strategy,
                    };
                    (cached, strategy)
                };

                let markdown = match cached {
                    Some(content) => {
                        tracing::info!(url = %url, "Cache hit");
                        content
                    }
                    None => {
                        let max_attempts = 3u32;
                        let mut visited: HashSet<Strategy> = HashSet::new();
                        let mut attempts = 0u32;
                        let mut current_strategy = initial_strategy;
                        let mut last_error: Option<String> = None;

                        loop {
                            if attempts >= max_attempts {
                                break format!(
                                    "[fetch error for {url}: all strategies exhausted: {}]",
                                    last_error.as_deref().unwrap_or("unknown")
                                );
                            }

                            visited.insert(current_strategy.clone());
                            attempts += 1;

                            let request = FetchRequest {
                                url: url.clone(),
                                headers: HashMap::new(),
                                timeout: Duration::from_secs(30),
                                follow_redirects: true,
                            };

                            let start = std::time::Instant::now();
                            match fetch_with_strategy(&current_strategy, request, None).await {
                                Ok(response) => {
                                    let latency_ms = start.elapsed().as_millis() as u64;
                                    tracing::info!(url = %url, strategy = ?current_strategy, latency_ms, "Batch fetch completed");

                                    let html = String::from_utf8_lossy(&response.body);
                                    let markdown = match extraction::extract(&html, &url, &response.content_type) {
                                        Ok(Some(e)) if !e.text_content.trim().is_empty() => e.text_content,
                                        _ => String::new(),
                                    };

                                    if is_content_valuable(&markdown) {
                                        let domain = url.split('/').nth(2).unwrap_or("unknown").to_string();
                                        let is_spa = spa_detect::detect_spa(&html).is_some();
                                        if let Ok(storage) = state.storage.lock() {
                                            store_and_record(
                                                &storage,
                                                &url,
                                                &domain,
                                                &current_strategy,
                                                &markdown,
                                                &response.content_type,
                                                latency_ms,
                                                response.status,
                                                is_spa,
                                            );
                                        }
                                        break markdown;
                                    } else {
                                        tracing::warn!(url = %url, strategy = ?current_strategy, "Content not valuable, trying next strategy");

                                        if current_strategy != Strategy::Headless {
                                            if let Some(framework) = spa_detect::detect_spa(&html) {
                                                tracing::info!(
                                                    url = %url,
                                                    detected_framework = ?framework,
                                                    "SPA detected, skipping to Headless"
                                                );
                                                visited.insert(Strategy::Headless);
                                                current_strategy = Strategy::Headless;
                                                continue;
                                            }
                                        }

                                        match next_strategy(
                                            &current_strategy,
                                            &visited,
                                            max_attempts,
                                            attempts,
                                            &FetchError::NetworkError("low quality content".into()),
                                        ) {
                                            Some(next) => {
                                                tracing::warn!(url = %url, from = ?current_strategy, to = ?next, "Falling back");
                                                current_strategy = next;
                                                continue;
                                            }
                                            None => {
                                                let mut md = markdown;
                                                let warning = format!(
                                                    "\n\n[Kaelo: unable to render JavaScript content for this URL. Tried: {} — none returned usable content.]",
                                                    visited.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ")
                                                );
                                                md.push_str(&warning);
                                                break md;
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!(url = %url, strategy = ?current_strategy, error = %e, "Batch fetch failed");
                                    let class = classify_error(&e);

                                    match class {
                                        ErrorClass::NotFound | ErrorClass::RateLimited => {
                                            return format!("[fetch error for {url}: {e}]");
                                        }
                                        ErrorClass::Transient | ErrorClass::Blocking => {
                                            last_error = Some(e.to_string());
                                            match next_strategy(
                                                &current_strategy,
                                                &visited,
                                                max_attempts,
                                                attempts,
                                                &e,
                                            ) {
                                                Some(next) => {
                                                    tracing::warn!(url = %url, from = ?current_strategy, to = ?next, "Falling back");
                                                    current_strategy = next;
                                                    continue;
                                                }
                                                None => {
                                                    return format!(
                                                        "[fetch error for {url}: all strategies exhausted: {}]",
                                                        last_error.as_deref().unwrap_or("unknown")
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                };

                apply_token_budget(markdown, token_budget)
            }));
        }

        let mut combined = String::new();
        for handle in handles {
            match handle.await {
                Ok(content) => {
                    if !combined.is_empty() {
                        combined.push_str("\n\n---\n\n");
                    }
                    combined.push_str(&content);
                }
                Err(e) => {
                    if !combined.is_empty() {
                        combined.push_str("\n\n---\n\n");
                    }
                    combined.push_str(&format!("[task error: {e}]"));
                }
            }
        }

        Ok(self.success_with_notice(combined))
    }

    #[tool(name = "ping", description = "Health check — returns pong")]
    fn ping(&self) -> String {
        format!(
            "{{\"status\":\"pong\",\"version\":\"{}\"}}",
            env!("CARGO_PKG_VERSION")
        )
    }

    #[tool(name = "cache_status", description = "Show cache statistics")]
    fn cache_status(&self) -> String {
        let storage = match self.state.storage.lock() {
            Ok(s) => s,
            Err(e) => return format!("Error acquiring storage lock: {e}"),
        };

        let route_cache = RouteCache::new(&storage);
        let content_cache = ContentCache::new(&storage);

        let route_count = route_cache.count_entries().unwrap_or(0);
        let stats =
            content_cache
                .stats()
                .unwrap_or(kaelo_core::storage::content_cache::CacheStats {
                    entry_count: 0,
                    total_size_bytes: 0,
                    top_domains: vec![],
                });

        format!(
            "Route cache: {} entries\nContent cache: {} entries ({} bytes)",
            route_count, stats.entry_count, stats.total_size_bytes
        )
    }

    #[tool(name = "cache_clear", description = "Clear all cached content")]
    fn cache_clear(&self) -> String {
        let storage = match self.state.storage.lock() {
            Ok(s) => s,
            Err(e) => return format!("Error acquiring storage lock: {e}"),
        };

        let route_cache = RouteCache::new(&storage);
        let content_cache = ContentCache::new(&storage);

        let route_count = route_cache.count_entries().unwrap_or(0);
        let content_count = content_cache.stats().map(|s| s.entry_count).unwrap_or(0);

        if let Err(e) = content_cache.clear_all() {
            return format!("Error clearing content cache: {e}");
        }

        if let Err(e) = storage.conn().execute("DELETE FROM domain_strategies", []) {
            return format!("Error clearing route cache: {e}");
        }

        format!(
            "Cache cleared: {} routes, {} content entries removed",
            route_count, content_count
        )
    }

    #[tool(name = "web_search", description = "Search the web using DuckDuckGo")]
    async fn web_search(
        &self,
        Parameters(params): Parameters<WebSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let max = params.max_results.unwrap_or(5).min(10) as usize;
        let query = params.query.clone();
        let fetch_content = params.fetch_content.unwrap_or(false);
        let backend = self.state.search_backend.as_deref().unwrap_or("ddg");

        tracing::info!(query = %query, max, backend, "Searching");
        let results = self.search_with_fallback(&query, max).await?;
        tracing::info!(query = %query, results = results.len(), "Search completed");

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No results found.".to_string(),
            )]));
        }

        if !fetch_content {
            let mut output = String::new();
            for (i, result) in results.iter().enumerate() {
                output.push_str(&format!(
                    "{}. {}\n   {}\n\n",
                    i + 1,
                    result.title,
                    result.url
                ));
            }
            return Ok(CallToolResult::success(vec![Content::text(output)]));
        }

        let fetch_limit = results.len().min(3);
        let mut output = String::new();

        for (i, result) in results.iter().take(fetch_limit).enumerate() {
            output.push_str(&format!("## {} - {}\n\n", i + 1, result.title));

            let cached = {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                check_content_cache(&storage, &result.url, None)
            };

            if let Some(content) = cached {
                let preview = truncate_preview(&content, 2000);
                output.push_str(&preview);
                output.push_str("\n\n");
            } else {
                let initial_strategy = {
                    let storage = self.state.storage.lock().map_err(internal_err)?;
                    resolve_strategy(&storage, &result.url).strategy
                };

                let max_attempts = 3u32;
                let mut visited: HashSet<Strategy> = HashSet::new();
                let mut attempts = 0u32;
                let mut current_strategy = initial_strategy;
                let mut last_error: Option<String> = None;
                let mut fetched_ok = false;

                loop {
                    if attempts >= max_attempts {
                        break;
                    }

                    visited.insert(current_strategy.clone());
                    attempts += 1;

                    let req = FetchRequest {
                        url: result.url.clone(),
                        headers: HashMap::new(),
                        timeout: Duration::from_secs(30),
                        follow_redirects: true,
                    };

                    let start = std::time::Instant::now();
                    match fetch_with_strategy(&current_strategy, req, None).await {
                        Ok(resp) => {
                            let latency_ms = start.elapsed().as_millis() as u64;
                            let body = String::from_utf8_lossy(&resp.body);
                            let is_spa = spa_detect::detect_spa(&body).is_some();
                            let extracted =
                                extraction::extract(&body, &result.url, &resp.content_type)
                                    .map_err(|e| internal_err(format!("extraction error: {e}")))?;

                            let content = extracted
                                .map(|e| e.text_content)
                                .unwrap_or_else(|| body.into_owned());

                            if is_content_valuable(&content) {
                                let domain = extract_domain(&result.url)
                                    .unwrap_or_else(|_| "unknown".to_string());
                                {
                                    let storage =
                                        self.state.storage.lock().map_err(internal_err)?;
                                    store_and_record(
                                        &storage,
                                        &result.url,
                                        &domain,
                                        &current_strategy,
                                        &content,
                                        &resp.content_type,
                                        latency_ms,
                                        resp.status,
                                        is_spa,
                                    );
                                }
                            }

                            let preview = truncate_preview(&content, 2000);
                            output.push_str(&preview);
                            output.push_str("\n\n");
                            fetched_ok = true;
                            break;
                        }
                        Err(e) => {
                            tracing::error!(url = %result.url, strategy = ?current_strategy, error = %e, "Search result fetch failed");
                            let class = classify_error(&e);

                            match class {
                                ErrorClass::NotFound | ErrorClass::RateLimited => {
                                    output.push_str(&format!("Failed to fetch: {e}\n\n"));
                                    fetched_ok = true;
                                    break;
                                }
                                ErrorClass::Transient | ErrorClass::Blocking => {
                                    last_error = Some(e.to_string());
                                    match next_strategy(
                                        &current_strategy,
                                        &visited,
                                        max_attempts,
                                        attempts,
                                        &e,
                                    ) {
                                        Some(next) => {
                                            tracing::warn!(url = %result.url, from = ?current_strategy, to = ?next, "Falling back");
                                            current_strategy = next;
                                            continue;
                                        }
                                        None => {
                                            output.push_str(&format!(
                                                "Failed to fetch (all strategies exhausted): {}\n\n",
                                                last_error.as_deref().unwrap_or("unknown")
                                            ));
                                            fetched_ok = true;
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !fetched_ok {
                    output.push_str(&format!(
                        "Failed to fetch (all strategies exhausted): {}\n\n",
                        last_error.as_deref().unwrap_or("unknown")
                    ));
                }
            }
        }

        for (i, result) in results.iter().skip(fetch_limit).enumerate() {
            output.push_str(&format!(
                "{}. {} — {}\n",
                fetch_limit + i + 1,
                result.title,
                result.url
            ));
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

#[tool_handler(name = "kaelo", version = "0.1.0")]
impl rmcp::handler::server::ServerHandler for KaeloServer {}

fn truncate_preview(content: &str, max_len: usize) -> String {
    if content.len() > max_len {
        format!("{} [...]", &content[..max_len])
    } else {
        content.to_string()
    }
}

fn apply_token_budget(mut content: String, token_budget: Option<u32>) -> String {
    if let Some(budget) = token_budget {
        if budget > 0 {
            let max_chars = (budget as usize) * 4;
            if content.len() > max_chars {
                content.truncate(max_chars);
                content.push_str("\n\n[... truncated to fit token budget]");
            }
        }
    }
    content
}

impl KaeloServer {
    async fn search_with_fallback(
        &self,
        query: &str,
        max_results: usize,
    ) -> Result<Vec<SearchResult>, ErrorData> {
        let use_searxng = self.state.search_backend.as_deref() == Some("searxng")
            && self.state.searxng_url.is_some();

        if use_searxng {
            if let Some(ref url) = self.state.searxng_url {
                match SearXngBackend::new(url.clone()) {
                    Ok(backend) => match backend.search(query, max_results).await {
                        Ok(results) if !results.is_empty() => return Ok(results),
                        Ok(_) => {
                            tracing::warn!("SearXNG returned empty results, falling back to DDG");
                        }
                        Err(e) => {
                            tracing::warn!("SearXNG error, falling back to DDG: {e}");
                        }
                    },
                    Err(e) => {
                        tracing::warn!("SearXNG init error, falling back to DDG: {e}");
                    }
                }
            }
        }

        let ddg = DuckDuckGoBackend::new().map_err(internal_err)?;
        ddg.search(query, max_results).await.map_err(internal_err)
    }
}

pub async fn serve_stdio(server: KaeloServer) -> Result<()> {
    tracing::info!("Starting Kaelo MCP server");

    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Serving error: {e:?}");
    })?;

    tracing::info!("Server connected, waiting for client messages...");
    service.waiting().await?;

    tracing::info!("Server shut down cleanly");
    Ok(())
}
