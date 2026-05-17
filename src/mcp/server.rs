use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::block_detect;
use crate::evidence;
use crate::extraction;
use crate::extraction::quality::is_content_valuable;
use crate::extraction::spa_detect;
use crate::extraction::token_estimator::TokenEstimator;
use crate::fetch::backend::FetchBackend;
use crate::fetch::backends::HttpSimple;
use crate::robots::{self, RobotsCache};
use crate::router::fallback::{classify_error, next_strategy, ErrorClass};
use crate::search::ddg::DuckDuckGoBackend;
use crate::search::searxng::SearXngBackend;
use crate::search::{SearchBackend, SearchResult};
use crate::ssrf;
use crate::storage::content_cache::ContentCache;
use crate::storage::route_cache::{FetchResult, RouteCache};
use crate::storage::Storage;
use crate::types::{
    Confidence, ExtractionMode, FetchDiagnostics, FetchError, FetchRequest, FetchResponse, Strategy,
};
use crate::update;
use anyhow::Result;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_handler, tool_router, transport::stdio, ErrorData, ServiceExt};

struct KaeloState {
    storage: Mutex<Storage>,
    searxng_url: Option<String>,
    search_backend: Option<String>,
    pending_update: Mutex<Option<String>>,
    robots_cache: RobotsCache,
    allow_private_networks: bool,
    respect_robots_txt: bool,
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
        let allow_private_networks = std::env::var("KAELO_ALLOW_PRIVATE_NETWORKS")
            .map(|v| matches!(v.as_str(), "true" | "1"))
            .unwrap_or(false);
        let respect_robots_txt = std::env::var("KAELO_RESPECT_ROBOTS_TXT")
            .map(|v| matches!(v.as_str(), "true" | "1"))
            .unwrap_or(false);

        let state = Arc::new(KaeloState {
            storage: Mutex::new(storage),
            searxng_url,
            search_backend,
            pending_update: Mutex::new(None),
            robots_cache: RobotsCache::new(),
            allow_private_networks,
            respect_robots_txt,
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

    fn build_response(&self, text: String, meta: &str) -> CallToolResult {
        self.build_response_with_diagnostics(text, meta, None, None)
    }

    fn build_response_with_diagnostics(
        &self,
        text: String,
        meta: &str,
        diagnostics: Option<&FetchDiagnostics>,
        evidence: Option<&crate::types::Evidence>,
    ) -> CallToolResult {
        let mut prefix_parts = Vec::new();
        if let Some(diag) = diagnostics {
            prefix_parts.push(format!("⚙ Kaelo: {}", diag.summary()));
        } else if !meta.is_empty() {
            prefix_parts.push(format!("⚙ Kaelo: {meta}"));
        }
        if let Some(ev) = evidence {
            prefix_parts.push(evidence::format_evidence_line(ev));
        }

        let full_text = if prefix_parts.is_empty() {
            text
        } else {
            format!("{}\n\n{}", prefix_parts.join("\n"), text)
        };

        let mut contents = vec![Content::text(full_text)];
        if let Some(notice) = self.get_update_notice() {
            contents.push(Content::text(notice));
        }
        let mut result = CallToolResult::success(contents);
        let mut meta_json = serde_json::Map::new();
        if !meta.is_empty() {
            meta_json.insert(
                "kaelo".to_string(),
                serde_json::Value::String(meta.to_string()),
            );
        }
        if let Some(diag) = diagnostics {
            meta_json.insert("diagnostics".to_string(), diag.to_meta());
        }
        if let Some(ev) = evidence {
            meta_json.insert(
                "evidence".to_string(),
                serde_json::to_value(ev).unwrap_or_default(),
            );
        }
        if !meta_json.is_empty() {
            result.meta = Some(rmcp::model::Meta(meta_json));
        }
        result
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
    /// Output format for the extracted content. Leave empty for Markdown (default).
    /// Options: "text", "html", "links", "json-ld", "tables", "metadata"
    #[serde(default)]
    pub format: Option<String>,
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
    /// Output format for all URLs. Options: "markdown" (default), "text", "html", "links", "json-ld", "tables", "metadata"
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct WebDiffParams {
    /// The URL to fetch and compare against the previous cached version
    pub url: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CacheSearchParams {
    /// Search query to match against cached URLs (supports partial matching)
    pub query: String,
    /// Maximum number of results to return (default 20, max 100)
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CacheDeleteParams {
    /// Delete all entries matching this domain (e.g. "example.com")
    #[serde(default)]
    pub domain: Option<String>,
    /// Delete the entry matching this exact URL
    #[serde(default)]
    pub url: Option<String>,
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
                use crate::fetch::backends::TlsImpersonation;
                TlsImpersonation::new()?.fetch(request).await
            }
            #[cfg(not(feature = "tls-impersonation"))]
            {
                HttpSimple::new()?.fetch(request).await
            }
        }
        Strategy::Headless => {
            use crate::fetch::backends::HeadlessBrowser;
            let browser = HeadlessBrowser::new().await?;
            browser.fetch_with_session(request, session_id).await
        }
        Strategy::PublicApi => HttpSimple::new()?.fetch(request).await,
        Strategy::YouTube => {
            use crate::fetch::backends::youtube::YouTubeBackend;
            YouTubeBackend::new()?.fetch(request).await
        }
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
    let content = match strategy {
        Some(s) => cache.get(url, s).ok().flatten().map(|c| c.content),
        None => cache.get_any(url).ok().flatten().map(|c| c.content),
    };

    // Validate cached content quality — reject poisoned/empty entries
    if let Some(ref content) = content {
        if !is_content_valuable(content) {
            tracing::debug!(url, "cached content failed quality check, invalidating");
            if let Some(s) = strategy {
                let _ = cache.invalidate(url, Some(s));
            }
            return None;
        }
    }

    content
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
        "YouTube" => Ok(Strategy::YouTube),
        other => Err(anyhow::anyhow!("unknown strategy: {other}")),
    }
}

async fn check_robots_allowed(url: &str, robots_cache: &RobotsCache) -> Result<(), ErrorData> {
    let parsed = url::Url::parse(url).map_err(|e| internal_err(format!("invalid URL: {e}")))?;
    let domain = parsed
        .host_str()
        .ok_or_else(|| internal_err("no host in URL"))?;
    let path = parsed.path();

    let robots_text = match robots_cache.get(domain) {
        Some(text) => text,
        None => {
            let robots_url = format!("{}://{}/robots.txt", parsed.scheme(), domain);
            let request = FetchRequest {
                url: robots_url,
                headers: HashMap::new(),
                timeout: Duration::from_secs(5),
                follow_redirects: true,
            };
            match HttpSimple::new().map(|b| async move { b.fetch(request).await }) {
                Ok(fut) => match fut.await {
                    Ok(resp) if resp.status < 400 => {
                        let text = String::from_utf8_lossy(&resp.body).to_string();
                        robots_cache.set(domain, text.clone());
                        text
                    }
                    _ => return Ok(()),
                },
                Err(_) => return Ok(()),
            }
        }
    };

    if !robots::is_allowed(path, &robots_text, "Kaelo") {
        return Err(internal_err(format!(
            "robots.txt disallows fetching {path} on {domain}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_diagnostics(
    strategy: &Strategy,
    cache_hit: bool,
    spa_detected: bool,
    js_rendered: bool,
    markdown: &str,
    extraction_mode: &ExtractionMode,
    fetch_latency_ms: u64,
    token_budget: Option<u32>,
    block_type: Option<crate::types::BlockType>,
) -> FetchDiagnostics {
    let content_length = markdown.len();
    let line_count = markdown.lines().count();
    let token_count = TokenEstimator::estimate(markdown) as usize;
    FetchDiagnostics {
        strategy_used: format!("{strategy:?}"),
        cache_hit,
        spa_detected,
        js_rendered,
        redirect_count: 0,
        content_length,
        line_count,
        token_count,
        token_budget: token_budget.map(|b| b as usize),
        confidence: if cache_hit {
            Confidence::High
        } else if js_rendered {
            Confidence::Medium
        } else {
            Confidence::High
        },
        extraction_mode: extraction_mode.as_str().to_string(),
        fetch_latency_ms,
        block_detected: block_type,
    }
}

struct FetchOutput {
    markdown: String,
    strategy_used: Strategy,
    cache_hit: bool,
    is_spa: bool,
    is_js_rendered: bool,
    latency_ms: u64,
    block_type: Option<crate::types::BlockType>,
    quality_warning: Option<String>,
}

enum FetchSingleError {
    Abort(String),
    Exhausted { last_error: String },
}

async fn fetch_with_fallback(
    state: &KaeloState,
    url: &str,
    mode: &ExtractionMode,
    strategy_override: Option<&Strategy>,
    session_id: Option<&str>,
    skip_cache: bool,
) -> Result<FetchOutput, FetchSingleError> {
    if !skip_cache {
        let storage = state
            .storage
            .lock()
            .map_err(|e| FetchSingleError::Abort(e.to_string()))?;
        if let Some(content) = check_content_cache(&storage, url, strategy_override) {
            tracing::info!(url, "Cache hit");
            return Ok(FetchOutput {
                markdown: content,
                strategy_used: strategy_override.cloned().unwrap_or(Strategy::HttpSimple),
                cache_hit: true,
                is_spa: false,
                is_js_rendered: false,
                latency_ms: 0,
                block_type: None,
                quality_warning: None,
            });
        }
    }

    let initial_strategy = match strategy_override {
        Some(s) => s.clone(),
        None => {
            let storage = state
                .storage
                .lock()
                .map_err(|e| FetchSingleError::Abort(e.to_string()))?;
            resolve_strategy(&storage, url).strategy
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
            url: url.to_string(),
            headers: HashMap::new(),
            timeout: Duration::from_secs(30),
            follow_redirects: true,
        };

        tracing::info!(
            url,
            strategy = ?current_strategy,
            attempt = attempts,
            "Fetching"
        );

        let start = std::time::Instant::now();
        match fetch_with_strategy(
            &current_strategy,
            request,
            session_id.map(|s| s.to_string()),
        )
        .await
        {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                tracing::info!(
                    url,
                    strategy = ?current_strategy,
                    latency_ms,
                    "Fetch completed"
                );

                let html = String::from_utf8_lossy(&response.body);
                let markdown =
                    match extraction::extract_with_mode(&html, url, &response.content_type, mode) {
                        Ok(Some(e)) if !e.text_content.trim().is_empty() => e.text_content,
                        _ => String::new(),
                    };

                let spa_framework = spa_detect::detect_spa(&html);
                let is_spa = spa_framework.is_some();
                let is_js_rendered = matches!(current_strategy, Strategy::Headless);
                let block_type =
                    block_detect::classify_response(response.status, &html, &response.headers)
                        .map(|b| b.to_block_type());

                if is_content_valuable(&markdown) {
                    let domain = extract_domain(url).unwrap_or_else(|_| "unknown".to_string());
                    {
                        let storage = state
                            .storage
                            .lock()
                            .map_err(|e| FetchSingleError::Abort(e.to_string()))?;
                        store_and_record(
                            &storage,
                            url,
                            &domain,
                            &current_strategy,
                            &markdown,
                            &response.content_type,
                            latency_ms,
                            response.status,
                            is_spa,
                        );
                    }
                    return Ok(FetchOutput {
                        markdown,
                        strategy_used: current_strategy,
                        cache_hit: false,
                        is_spa,
                        is_js_rendered,
                        latency_ms,
                        block_type,
                        quality_warning: None,
                    });
                }

                tracing::warn!(
                    url,
                    strategy = ?current_strategy,
                    "Content not valuable enough to cache, trying next strategy"
                );

                if is_spa && !visited.contains(&Strategy::Headless) {
                    tracing::info!(
                        url,
                        spa = ?spa_framework,
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
                            url,
                            from = ?current_strategy,
                            to = ?next,
                            "Falling back"
                        );
                        current_strategy = next;
                        continue;
                    }
                    None => {
                        let warning = format!(
                            "unable to render JavaScript content for {}. Tried: {} — none returned usable content.",
                            url,
                            visited
                                .iter()
                                .map(|s| format!("{s:?}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        );
                        return Ok(FetchOutput {
                            markdown,
                            strategy_used: current_strategy,
                            cache_hit: false,
                            is_spa,
                            is_js_rendered,
                            latency_ms,
                            block_type,
                            quality_warning: Some(warning),
                        });
                    }
                }
            }
            Err(e) => {
                tracing::error!(
                    url,
                    strategy = ?current_strategy,
                    error = %e,
                    "Fetch failed"
                );
                let class = classify_error(&e);

                match class {
                    ErrorClass::NotFound | ErrorClass::RateLimited => {
                        return Err(FetchSingleError::Abort(e.to_string()));
                    }
                    ErrorClass::Transient | ErrorClass::Blocking | ErrorClass::Blocked(_) => {
                        last_error = Some(e.to_string());
                        match next_strategy(&current_strategy, &visited, max_attempts, attempts, &e)
                        {
                            Some(next) => {
                                tracing::warn!(
                                    url,
                                    from = ?current_strategy,
                                    to = ?next,
                                    "Falling back"
                                );
                                current_strategy = next;
                                continue;
                            }
                            None => {
                                return Err(FetchSingleError::Exhausted {
                                    last_error: last_error.unwrap_or_else(|| "unknown".to_string()),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Err(FetchSingleError::Exhausted {
        last_error: last_error.unwrap_or_else(|| "unknown".to_string()),
    })
}

#[tool_router]
impl KaeloServer {
    #[tool(
        name = "web_fetch",
        description = "Fetch a URL and return clean Markdown content. IMPORTANT: always report the first line of the response (⚙ Kaelo: ...) to the user — it shows how the content was obtained (strategy used, latency, or cached)."
    )]
    async fn web_fetch(
        &self,
        Parameters(params): Parameters<WebFetchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let no_cache = params.no_cache.unwrap_or(false);
        let mode = params
            .format
            .as_deref()
            .map(ExtractionMode::parse_mode)
            .unwrap_or_default();

        if !self.state.allow_private_networks {
            if let Err(e) = ssrf::check_ssrf(&params.url) {
                return Err(internal_err(format!("SSRF blocked: {e}")));
            }
        }

        if self.state.respect_robots_txt {
            check_robots_allowed(&params.url, &self.state.robots_cache).await?;
        }

        let explicit_strategy = params
            .strategy
            .as_deref()
            .map(|s| parse_strategy_name(s).unwrap_or(Strategy::HttpSimple));

        if no_cache {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            let cache = ContentCache::new(&storage);
            let _ = cache.invalidate(&params.url, explicit_strategy.as_ref());
        }

        match fetch_with_fallback(
            &self.state,
            &params.url,
            &mode,
            explicit_strategy.as_ref(),
            params.session.as_deref(),
            no_cache,
        )
        .await
        {
            Ok(output) if output.cache_hit => {
                let result = apply_token_budget(output.markdown, params.token_budget);
                let diag = build_diagnostics(
                    &output.strategy_used,
                    true,
                    false,
                    false,
                    &result,
                    &mode,
                    0,
                    params.token_budget,
                    None,
                );
                let meta = format!("{:?}, 0ms, cache HIT", output.strategy_used);
                Ok(self.build_response_with_diagnostics(result, &meta, Some(&diag), None))
            }
            Ok(output) if output.quality_warning.is_some() => {
                let warning = output.quality_warning.as_ref().unwrap().clone();
                let diag = build_diagnostics(
                    &output.strategy_used,
                    false,
                    false,
                    false,
                    &output.markdown,
                    &ExtractionMode::Markdown,
                    0,
                    params.token_budget,
                    None,
                );
                let meta = format!("{:?} — quality warning: {warning}", output.strategy_used,);
                let mut result = apply_token_budget(output.markdown, params.token_budget);
                result.push_str(&format!("\n\n[Kaelo: {warning}]"));
                Ok(self.build_response_with_diagnostics(result, &meta, Some(&diag), None))
            }
            Ok(output) => {
                let _domain = extract_domain(&params.url).unwrap_or_else(|_| "unknown".to_string());
                let ev = evidence::build_evidence(
                    &output.markdown,
                    &params.url,
                    &format!("{:?}", output.strategy_used),
                    output.markdown.len(),
                    output.block_type.as_ref(),
                );
                {
                    let storage = self.state.storage.lock().map_err(internal_err)?;
                    let diag_json = serde_json::to_string(&build_diagnostics(
                        &output.strategy_used,
                        false,
                        output.is_spa,
                        output.is_js_rendered,
                        &output.markdown,
                        &mode,
                        output.latency_ms,
                        params.token_budget,
                        output.block_type.clone(),
                    ))
                    .ok();
                    if let Err(e) = evidence::store_evidence(&storage, &ev, diag_json.as_deref()) {
                        tracing::warn!(url = %params.url, "Failed to store evidence: {e}");
                    }
                }
                let diag = build_diagnostics(
                    &output.strategy_used,
                    false,
                    output.is_spa,
                    output.is_js_rendered,
                    &output.markdown,
                    &mode,
                    output.latency_ms,
                    params.token_budget,
                    output.block_type,
                );
                let result = apply_token_budget(output.markdown, params.token_budget);
                let meta = format!("{:?}, {}ms", output.strategy_used, output.latency_ms);
                Ok(self.build_response_with_diagnostics(result, &meta, Some(&diag), Some(&ev)))
            }
            Err(FetchSingleError::Abort(msg)) => Err(internal_err(msg)),
            Err(FetchSingleError::Exhausted { last_error }) => Err(internal_err(format!(
                "All fetch strategies exhausted for {}: {}",
                params.url, last_error
            ))),
        }
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

        let mode = params
            .format
            .as_deref()
            .map(ExtractionMode::parse_mode)
            .unwrap_or_default();

        if !self.state.allow_private_networks {
            for url in &params.urls {
                if let Err(e) = ssrf::check_ssrf(url) {
                    return Err(internal_err(format!("SSRF blocked for {url}: {e}")));
                }
            }
        }

        if self.state.respect_robots_txt {
            for url in &params.urls {
                check_robots_allowed(url, &self.state.robots_cache).await?;
            }
        }

        tracing::info!(urls = ?params.urls, "Batch fetching {} URLs", params.urls.len());

        let url_count = params.urls.len();
        let state = self.state.clone();
        let strategy_name = params.strategy;
        let token_budget = params.token_budget;
        let format_mode = mode.clone();

        let mut handles: Vec<tokio::task::JoinHandle<String>> = Vec::new();

        for url in params.urls {
            let state = state.clone();
            let strategy_name = strategy_name.clone();
            let format_mode = format_mode.clone();

            handles.push(tokio::spawn(async move {
                let explicit_strategy = strategy_name
                    .as_deref()
                    .map(|s| parse_strategy_name(s).unwrap_or(Strategy::HttpSimple));

                match fetch_with_fallback(
                    &state,
                    &url,
                    &format_mode,
                    explicit_strategy.as_ref(),
                    None,
                    false,
                )
                .await
                {
                    Ok(mut output) => {
                        if let Some(warning) = output.quality_warning.take() {
                            output.markdown.push_str(&format!("\n\n[Kaelo: {warning}]"));
                        }
                        apply_token_budget(output.markdown, token_budget)
                    }
                    Err(FetchSingleError::Abort(msg)) => {
                        format!("[fetch error for {url}: {msg}]")
                    }
                    Err(FetchSingleError::Exhausted { last_error }) => {
                        format!("[fetch error for {url}: all strategies exhausted: {last_error}]")
                    }
                }
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

        let meta = format!("{} URLs fetched", url_count);
        let diag = FetchDiagnostics {
            strategy_used: "batch".to_string(),
            cache_hit: false,
            spa_detected: false,
            js_rendered: false,
            redirect_count: 0,
            content_length: combined.len(),
            line_count: combined.lines().count(),
            token_count: TokenEstimator::estimate(&combined) as usize,
            token_budget: token_budget.map(|b| b as usize),
            confidence: Confidence::Medium,
            extraction_mode: mode.as_str().to_string(),
            fetch_latency_ms: 0,
            block_detected: None,
        };
        Ok(self.build_response_with_diagnostics(combined, &meta, Some(&diag), None))
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
        let stats = content_cache
            .stats()
            .unwrap_or(crate::storage::content_cache::CacheStats {
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

    #[tool(
        name = "cache_search",
        description = "Semantic search across all cached content using keyword query on URL and domain"
    )]
    fn cache_search(&self, Parameters(params): Parameters<CacheSearchParams>) -> String {
        let storage = match self.state.storage.lock() {
            Ok(s) => s,
            Err(e) => return format!("Error acquiring storage lock: {e}"),
        };

        let content_cache = ContentCache::new(&storage);
        let limit = params.limit.unwrap_or(20).min(100);

        match content_cache.search_entries(&params.query, limit) {
            Ok(entries) if entries.is_empty() => {
                format!("No cache entries matching '{}'.", params.query)
            }
            Ok(entries) => {
                let mut output = format!(
                    "Found {} entries matching '{}':\n\n",
                    entries.len(),
                    params.query
                );
                for (i, entry) in entries.iter().enumerate() {
                    output.push_str(&format!(
                        "{}. `{}{}` [{}]\n   Size: {} bytes | Strategy: {}\n\n",
                        i + 1,
                        entry.url,
                        if entry.content_type.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", entry.content_type)
                        },
                        entry.created_at,
                        entry.original_size,
                        entry.strategy,
                    ));
                }
                output
            }
            Err(e) => format!("Error searching cache: {e}"),
        }
    }

    #[tool(
        name = "cache_stats",
        description = "Returns total entries, total size, top 10 domains, oldest and newest timestamps"
    )]
    fn cache_stats(&self) -> String {
        let storage = match self.state.storage.lock() {
            Ok(s) => s,
            Err(e) => return format!("Error acquiring storage lock: {e}"),
        };

        let content_cache = ContentCache::new(&storage);

        match content_cache.detailed_stats() {
            Ok(stats) => {
                let mut output = String::new();
                output.push_str(&format!("**Entries:** {}\n", stats.entry_count));
                output.push_str(&format!(
                    "**Total size:** {} bytes ({:.1} KB)\n",
                    stats.total_size_bytes,
                    stats.total_size_bytes as f64 / 1024.0
                ));

                if let Some(ref oldest) = stats.oldest_at {
                    output.push_str(&format!("**Oldest:** {}\n", oldest));
                }
                if let Some(ref newest) = stats.newest_at {
                    output.push_str(&format!("**Newest:** {}\n", newest));
                }

                if !stats.top_domains.is_empty() {
                    output.push_str("\n**Top domains:**\n");
                    for (domain, count) in &stats.top_domains {
                        output.push_str(&format!("- {} ({} entries)\n", domain, count));
                    }
                }

                output
            }
            Err(e) => format!("Error getting cache stats: {e}"),
        }
    }

    #[tool(
        name = "cache_delete",
        description = "Delete matching cache entries by domain or URL. At least one of domain or url must be provided."
    )]
    fn cache_delete(&self, Parameters(params): Parameters<CacheDeleteParams>) -> String {
        if params.domain.is_none() && params.url.is_none() {
            return "Error: at least one of 'domain' or 'url' must be provided.".to_string();
        }

        let storage = match self.state.storage.lock() {
            Ok(s) => s,
            Err(e) => return format!("Error acquiring storage lock: {e}"),
        };

        let content_cache = ContentCache::new(&storage);
        let mut total_deleted: u64 = 0;

        if let Some(ref url) = params.url {
            match content_cache.delete_by_url(url) {
                Ok(count) => total_deleted += count,
                Err(e) => return format!("Error deleting by URL: {e}"),
            }
        }

        if let Some(ref domain) = params.domain {
            match content_cache.delete_by_domain(domain) {
                Ok(count) => total_deleted += count,
                Err(e) => return format!("Error deleting by domain: {e}"),
            }
        }

        format!("Deleted {} cache entries.", total_deleted)
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
                if !self.state.allow_private_networks {
                    if let Err(e) = ssrf::check_ssrf(&result.url) {
                        output.push_str(&format!(
                            "{}. {}\n   {} (SSRF blocked: {e})\n\n",
                            i + 1,
                            result.title,
                            result.url
                        ));
                        continue;
                    }
                }

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
                                ErrorClass::Transient
                                | ErrorClass::Blocking
                                | ErrorClass::Blocked(_) => {
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

    #[tool(
        name = "web_diff",
        description = "Fetch a URL and compare the current version against the previously cached version, showing what changed."
    )]
    async fn web_diff(
        &self,
        Parameters(params): Parameters<WebDiffParams>,
    ) -> Result<CallToolResult, ErrorData> {
        if !self.state.allow_private_networks {
            if let Err(e) = ssrf::check_ssrf(&params.url) {
                return Err(internal_err(format!("SSRF blocked: {e}")));
            }
        }

        if self.state.respect_robots_txt {
            check_robots_allowed(&params.url, &self.state.robots_cache).await?;
        }

        let initial_strategy = {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            resolve_strategy(&storage, &params.url).strategy
        };

        let max_attempts = 3u32;
        let mut visited: HashSet<Strategy> = HashSet::new();
        let mut attempts = 0u32;
        let mut current_strategy = initial_strategy;
        let mut last_error: Option<String> = None;
        let mut fetched_markdown: Option<String> = None;
        let mut used_strategy = current_strategy.clone();

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
                "web_diff: fetching"
            );

            let start = std::time::Instant::now();
            match fetch_with_strategy(&current_strategy, request, None).await {
                Ok(response) => {
                    let latency_ms = start.elapsed().as_millis() as u64;
                    let html = String::from_utf8_lossy(&response.body);
                    let mode = ExtractionMode::Markdown;
                    let extracted = extraction::extract_with_mode(
                        &html,
                        &params.url,
                        &response.content_type,
                        &mode,
                    )
                    .map_err(|e| internal_err(format!("extraction error: {e}")))?;

                    let markdown = match extracted {
                        Some(e) if !e.text_content.trim().is_empty() => e.text_content,
                        _ => String::new(),
                    };

                    if is_content_valuable(&markdown) {
                        let markdown_clone = markdown.clone();
                        fetched_markdown = Some(markdown);
                        used_strategy = current_strategy.clone();

                        let domain =
                            extract_domain(&params.url).unwrap_or_else(|_| "unknown".to_string());
                        {
                            let storage = self.state.storage.lock().map_err(internal_err)?;
                            store_and_record(
                                &storage,
                                &params.url,
                                &domain,
                                &current_strategy,
                                &markdown_clone,
                                &response.content_type,
                                latency_ms,
                                response.status,
                                spa_detect::detect_spa(&html).is_some(),
                            );
                        }
                        break;
                    }

                    let is_spa = spa_detect::detect_spa(&html).is_some();
                    if is_spa && !visited.contains(&Strategy::Headless) {
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
                            current_strategy = next;
                            continue;
                        }
                        None => break,
                    }
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                    let class = classify_error(&e);
                    match class {
                        ErrorClass::NotFound | ErrorClass::RateLimited => break,
                        _ => {
                            match next_strategy(
                                &current_strategy,
                                &visited,
                                max_attempts,
                                attempts,
                                &e,
                            ) {
                                Some(next) => {
                                    current_strategy = next;
                                    continue;
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
        }

        let markdown = match fetched_markdown {
            Some(m) => m,
            None => {
                return Err(internal_err(format!(
                    "All fetch strategies exhausted for {}: {}",
                    params.url,
                    last_error.as_deref().unwrap_or("unknown")
                )));
            }
        };

        let new_hash = evidence::compute_hash(&markdown);

        let previous = {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            storage
                .get_page_history(&params.url, 1)
                .map_err(internal_err)?
        };

        if previous.is_empty() {
            {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                storage
                    .store_page_history(
                        &params.url,
                        &new_hash,
                        &markdown,
                        &format!("{:?}", used_strategy),
                    )
                    .map_err(internal_err)?;
            }

            let page_diff = crate::types::PageDiff {
                url: params.url.clone(),
                old_hash: None,
                new_hash,
                added_lines: 0,
                removed_lines: 0,
                unchanged_lines: 0,
                similarity_percent: 100.0,
                diff_text: String::new(),
            };

            let meta_label = "first snapshot";
            let mut meta_json = serde_json::Map::new();
            meta_json.insert(
                "kaelo".to_string(),
                serde_json::Value::String(meta_label.to_string()),
            );
            meta_json.insert(
                "page_diff".to_string(),
                serde_json::to_value(&page_diff).unwrap_or_default(),
            );
            let mut result = self.build_response(
                "📊 First snapshot stored. No previous version to compare.".to_string(),
                meta_label,
            );
            result.meta = Some(rmcp::model::Meta(meta_json));
            return Ok(result);
        }

        let (old_hash, old_body, _old_strategy, _old_fetched_at) = &previous[0];

        let new_lines: Vec<&str> = markdown.lines().collect();
        let old_lines: Vec<&str> = old_body.lines().collect();

        let old_set: std::collections::HashSet<&str> = old_lines.iter().copied().collect();
        let new_set: std::collections::HashSet<&str> = new_lines.iter().copied().collect();

        let added: Vec<&&str> = new_lines
            .iter()
            .filter(|l| !old_set.contains(**l))
            .collect();
        let removed: Vec<&&str> = old_lines
            .iter()
            .filter(|l| !new_set.contains(**l))
            .collect();

        let unchanged_count = new_lines.iter().filter(|l| old_set.contains(**l)).count();
        let total = new_lines.len().max(old_lines.len());
        let similarity = if total == 0 {
            100.0
        } else {
            (unchanged_count as f64 / total as f64 * 100.0 * 100.0).round() / 100.0
        };

        let mut diff_text = String::new();
        for line in &added {
            diff_text.push_str(&format!("+ {}\n", line));
        }
        for line in &removed {
            diff_text.push_str(&format!("- {}\n", line));
        }

        let added_count = added.len() as u32;
        let removed_count = removed.len() as u32;
        let unchanged_count_u32 = unchanged_count as u32;

        {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            storage
                .store_page_history(
                    &params.url,
                    &new_hash,
                    &markdown,
                    &format!("{:?}", used_strategy),
                )
                .map_err(internal_err)?;
        }

        let page_diff = crate::types::PageDiff {
            url: params.url.clone(),
            old_hash: Some(old_hash.clone()),
            new_hash,
            added_lines: added_count,
            removed_lines: removed_count,
            unchanged_lines: unchanged_count_u32,
            similarity_percent: similarity,
            diff_text: diff_text.clone(),
        };

        let summary = format!(
            "📊 Diff: +{} lignes, -{} lignes | contenu inchangé à {}%",
            added_count, removed_count, similarity
        );

        let mut output = format!("{}\n\n", summary);
        if !diff_text.is_empty() {
            output.push_str("```diff\n");
            output.push_str(&diff_text);
            output.push_str("```\n");
        }

        let meta_label = &summary[5..];
        let mut output = format!("{}\n\n", summary);
        if !diff_text.is_empty() {
            output.push_str("```diff\n");
            output.push_str(&diff_text);
            output.push_str("```\n");
        }

        let mut meta_json = serde_json::Map::new();
        meta_json.insert(
            "kaelo".to_string(),
            serde_json::Value::String(meta_label.to_string()),
        );
        meta_json.insert(
            "page_diff".to_string(),
            serde_json::to_value(&page_diff).unwrap_or_default(),
        );
        let mut result = self.build_response(output, meta_label);
        result.meta = Some(rmcp::model::Meta(meta_json));
        Ok(result)
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

#[cfg(test)]
mod web_diff_tests {
    use super::*;
    use crate::evidence;

    fn compute_diff(old: &str, new: &str) -> (u32, u32, u32, f64) {
        let new_lines: Vec<&str> = new.lines().collect();
        let old_lines: Vec<&str> = old.lines().collect();
        let old_set: std::collections::HashSet<&str> = old_lines.iter().copied().collect();
        let new_set: std::collections::HashSet<&str> = new_lines.iter().copied().collect();
        let added = new_lines.iter().filter(|l| !old_set.contains(**l)).count() as u32;
        let removed = old_lines.iter().filter(|l| !new_set.contains(**l)).count() as u32;
        let unchanged = new_lines.iter().filter(|l| old_set.contains(**l)).count() as u32;
        let total = new_lines.len().max(old_lines.len());
        let similarity = if total == 0 {
            100.0
        } else {
            (unchanged as f64 / total as f64 * 100.0 * 100.0).round() / 100.0
        };
        (added, removed, unchanged, similarity)
    }

    #[test]
    fn test_diff_first_visit_no_previous() {
        let storage = Storage::open(":memory:").unwrap();
        let history = storage.get_page_history("https://example.com", 1).unwrap();
        assert!(
            history.is_empty(),
            "No history should exist for first visit"
        );
    }

    #[test]
    fn test_diff_store_and_retrieve() {
        let storage = Storage::open(":memory:").unwrap();
        let content = "line 1\nline 2\nline 3";
        let hash = evidence::compute_hash(content);

        storage
            .store_page_history("https://example.com", &hash, content, "HttpSimple")
            .unwrap();

        let history = storage.get_page_history("https://example.com", 1).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].0, hash);
        assert_eq!(history[0].1, content);
    }

    #[test]
    fn test_diff_changed_content() {
        let (added, removed, unchanged, similarity) =
            compute_diff("line 1\nline 2\nline 3", "line 1\nline 2 modified\nline 4");
        assert_eq!(
            added, 2,
            "Should have 2 added lines (modified line 2 + new line 4)"
        );
        assert_eq!(
            removed, 2,
            "Should have 2 removed lines (original line 2 + original line 3)"
        );
        assert_eq!(unchanged, 1, "Only line 1 unchanged");
        assert!(similarity < 50.0, "Similarity should be below 50%");
    }

    #[test]
    fn test_diff_unchanged_content() {
        let (added, removed, unchanged, similarity) =
            compute_diff("line 1\nline 2\nline 3", "line 1\nline 2\nline 3");
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
        assert_eq!(unchanged, 3);
        assert_eq!(similarity, 100.0);
    }

    #[test]
    fn test_diff_empty_to_content() {
        let (added, removed, unchanged, _) = compute_diff("", "new line\nanother line");
        assert_eq!(added, 2);
        assert_eq!(removed, 0);
        assert_eq!(unchanged, 0);
    }

    #[test]
    fn test_diff_content_to_empty() {
        let (added, removed, unchanged, _) = compute_diff("old line\nanother old", "");
        assert_eq!(added, 0);
        assert_eq!(removed, 2);
        assert_eq!(unchanged, 0);
    }

    #[test]
    fn test_diff_max_versions_per_url() {
        let storage = Storage::open(":memory:").unwrap();
        for i in 0..12 {
            let content = format!("version {}", i);
            let hash = evidence::compute_hash(&content);
            storage
                .store_page_history(
                    "https://example.com/versions",
                    &hash,
                    &content,
                    "HttpSimple",
                )
                .unwrap();
        }
        let history = storage
            .get_page_history("https://example.com/versions", 20)
            .unwrap();
        assert!(
            history.len() <= 10,
            "Should cap at 10 versions, got {}",
            history.len()
        );
    }

    #[test]
    fn test_diff_multiple_versions_stored() {
        let storage = Storage::open(":memory:").unwrap();
        let content_v1 = "version 1";
        let hash_v1 = evidence::compute_hash(content_v1);
        storage
            .store_page_history(
                "https://example.com/order",
                &hash_v1,
                content_v1,
                "HttpSimple",
            )
            .unwrap();

        let content_v2 = "version 2";
        let hash_v2 = evidence::compute_hash(content_v2);
        storage
            .store_page_history(
                "https://example.com/order",
                &hash_v2,
                content_v2,
                "HttpSimple",
            )
            .unwrap();

        let history = storage
            .get_page_history("https://example.com/order", 10)
            .unwrap();
        assert_eq!(history.len(), 2, "Should have 2 versions");
        let hashes: Vec<&str> = history.iter().map(|(h, _, _, _)| h.as_str()).collect();
        assert!(hashes.contains(&hash_v1.as_str()), "Should contain v1 hash");
        assert!(hashes.contains(&hash_v2.as_str()), "Should contain v2 hash");
    }

    #[test]
    fn test_page_diff_struct_serialization() {
        let diff = crate::types::PageDiff {
            url: "https://example.com".to_string(),
            old_hash: Some("abc123".to_string()),
            new_hash: "def456".to_string(),
            added_lines: 3,
            removed_lines: 1,
            unchanged_lines: 10,
            similarity_percent: 76.92,
            diff_text: "+ new line\n- old line\n".to_string(),
        };
        let json = serde_json::to_value(&diff).unwrap();
        assert_eq!(json["url"], "https://example.com");
        assert_eq!(json["added_lines"], 3);
        assert_eq!(json["similarity_percent"], 76.92);
    }
}
