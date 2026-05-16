use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use kaelo_core::extraction;
use kaelo_core::search::ddg::DuckDuckGoBackend;
use kaelo_core::search::searxng::SearXngBackend;
use kaelo_core::search::{SearchBackend, SearchResult};
use kaelo_core::storage::content_cache::ContentCache;
use kaelo_core::storage::route_cache::{FetchResult, RouteCache};
use kaelo_core::storage::Storage;
use kaelo_core::types::{FetchError, FetchRequest, FetchResponse, Strategy};
use kaelo_fetch::backend::FetchBackend;
use kaelo_fetch::backends::HttpSimple;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_handler, tool_router, transport::stdio, ErrorData, ServiceExt};

struct KaeloState {
    storage: Mutex<Storage>,
    searxng_url: Option<String>,
    search_backend: Option<String>,
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
        let state = KaeloState {
            storage: Mutex::new(storage),
            searxng_url,
            search_backend,
        };
        Self {
            state: Arc::new(state),
        }
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
    domain: String,
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
    ResolvedStrategy { strategy, domain }
}

fn check_content_cache(storage: &Storage, url: &str) -> Option<String> {
    let cache = ContentCache::new(storage);
    cache.get(url).ok().flatten().map(|c| c.content)
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
) {
    let cache = ContentCache::new(storage);
    let _ = cache.put(url, markdown, content_type, Duration::from_secs(3600));

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

        let (cached, strategy) = {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            let cached = if !no_cache {
                check_content_cache(&storage, &params.url)
            } else {
                None
            };

            let strategy = match &params.strategy {
                Some(s) => parse_strategy_name(s).unwrap_or(Strategy::HttpSimple),
                None => resolve_strategy(&storage, &params.url).strategy,
            };

            (cached, strategy)
        };

        let markdown = if let Some(content) = cached {
            tracing::info!(url = %params.url, "Cache hit");
            content
        } else {
            tracing::info!(url = %params.url, strategy = ?strategy, "Cache miss, fetching");
            let request = FetchRequest {
                url: params.url.clone(),
                headers: HashMap::new(),
                timeout: Duration::from_secs(30),
                follow_redirects: true,
            };

            let start = std::time::Instant::now();
            let response = fetch_with_strategy(&strategy, request, params.session.clone())
                .await
                .map_err(|e| {
                    tracing::error!(url = %params.url, strategy = ?strategy, error = %e, "Fetch failed");
                    internal_err(e)
                })?;
            let latency_ms = start.elapsed().as_millis() as u64;
            tracing::info!(url = %params.url, strategy = ?strategy, latency_ms, "Fetch completed");

            let html = String::from_utf8_lossy(&response.body);
            let extracted = extraction::extract(&html, &params.url, &response.content_type)
                .map_err(|e| internal_err(format!("extraction error: {e}")))?;

            let markdown = extracted
                .map(|e| e.text_content)
                .unwrap_or_else(|| html.into_owned());

            let domain = params
                .url
                .split('/')
                .nth(2)
                .unwrap_or("unknown")
                .to_string();

            {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                store_and_record(
                    &storage,
                    &params.url,
                    &domain,
                    &strategy,
                    &markdown,
                    &response.content_type,
                    latency_ms,
                    response.status,
                );
            }

            markdown
        };

        let mut result = markdown;
        if let Some(budget) = params.token_budget {
            if budget > 0 {
                let max_chars = (budget as usize) * 4;
                if result.len() > max_chars {
                    result.truncate(max_chars);
                    result.push_str("\n\n[... truncated to fit token budget]");
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(result)]))
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
                let (cached, strategy) = {
                    let storage = match state.storage.lock() {
                        Ok(s) => s,
                        Err(e) => return format!("[error: {e}]"),
                    };
                    let cached = check_content_cache(&storage, &url);
                    let strategy = match &strategy_name {
                        Some(s) => parse_strategy_name(s).unwrap_or(Strategy::HttpSimple),
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
                        let request = FetchRequest {
                            url: url.clone(),
                            headers: HashMap::new(),
                            timeout: Duration::from_secs(30),
                            follow_redirects: true,
                        };

                        let start = std::time::Instant::now();
                        let response = match fetch_with_strategy(&strategy, request, None).await {
                            Ok(r) => r,
                            Err(e) => {
                                tracing::error!(url = %url, strategy = ?strategy, error = %e, "Batch fetch failed");
                                return format!("[fetch error for {url}: {e}]");
                            }
                        };
                        let latency_ms = start.elapsed().as_millis() as u64;
                        tracing::info!(url = %url, strategy = ?strategy, latency_ms, "Batch fetch completed");

                        let html = String::from_utf8_lossy(&response.body);
                        let extracted =
                            match extraction::extract(&html, &url, &response.content_type) {
                                Ok(Some(e)) => e.text_content,
                                Ok(None) => html.into_owned(),
                                Err(e) => return format!("[extraction error for {url}: {e}]"),
                            };

                        let domain = url.split('/').nth(2).unwrap_or("unknown").to_string();

                        {
                            if let Ok(storage) = state.storage.lock() {
                                store_and_record(
                                    &storage,
                                    &url,
                                    &domain,
                                    &strategy,
                                    &extracted,
                                    &response.content_type,
                                    latency_ms,
                                    response.status,
                                );
                            }
                        }

                        extracted
                    }
                };

                let mut result = markdown;
                if let Some(budget) = token_budget {
                    if budget > 0 {
                        let max_chars = (budget as usize) * 4;
                        if result.len() > max_chars {
                            result.truncate(max_chars);
                            result.push_str("\n\n[... truncated to fit token budget]");
                        }
                    }
                }

                result
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

        Ok(CallToolResult::success(vec![Content::text(combined)]))
    }

    #[tool(name = "ping", description = "Health check — returns pong")]
    fn ping(&self) -> String {
        "pong".to_string()
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

            let (cached, resolved) = {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                let cached = check_content_cache(&storage, &result.url);
                let resolved = resolve_strategy(&storage, &result.url);
                (cached, resolved)
            };

            if let Some(content) = cached {
                let preview = truncate_preview(&content, 2000);
                output.push_str(&preview);
                output.push_str("\n\n");
            } else {
                let req = FetchRequest {
                    url: result.url.clone(),
                    headers: HashMap::new(),
                    timeout: Duration::from_secs(30),
                    follow_redirects: true,
                };

                let start = std::time::Instant::now();
                match fetch_with_strategy(&resolved.strategy, req, None).await {
                    Ok(resp) => {
                        let latency_ms = start.elapsed().as_millis() as u64;
                        let body = String::from_utf8_lossy(&resp.body);
                        let extracted = extraction::extract(&body, &result.url, &resp.content_type)
                            .map_err(|e| internal_err(format!("extraction error: {e}")))?;

                        let content = extracted
                            .map(|e| e.text_content)
                            .unwrap_or_else(|| body.into_owned());

                        {
                            let storage = self.state.storage.lock().map_err(internal_err)?;
                            store_and_record(
                                &storage,
                                &result.url,
                                &resolved.domain,
                                &resolved.strategy,
                                &content,
                                &resp.content_type,
                                latency_ms,
                                resp.status,
                            );
                        }

                        let preview = truncate_preview(&content, 2000);
                        output.push_str(&preview);
                        output.push_str("\n\n");
                    }
                    Err(e) => {
                        tracing::error!(url = %result.url, error = %e, "Search result fetch failed");
                        output.push_str(&format!("Failed to fetch: {e}\n\n"));
                    }
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
