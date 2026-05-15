//! Production MCP server for Kaelo.
//!
//! Uses `rmcp` with stdio transport. Provides tools for web fetching,
//! searching, and cache management.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use kaelo_core::extraction;
use kaelo_core::storage::content_cache::ContentCache;
use kaelo_core::storage::route_cache::{FetchResult, RouteCache};
use kaelo_core::storage::Storage;
use kaelo_core::types::{FetchError, FetchRequest, FetchResponse, Strategy};
use kaelo_fetch::backend::FetchBackend;
use kaelo_fetch::backends::HttpSimple;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use rmcp::{tool, tool_router, transport::stdio, ErrorData, ServiceExt};
use tracing_subscriber::EnvFilter;

struct KaeloState {
    storage: Mutex<Storage>,
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
        let state = KaeloState {
            storage: Mutex::new(storage),
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
    /// Maximum token budget (approximate character count / 4). 0 = no limit.
    #[serde(default)]
    pub token_budget: Option<u32>,
    /// CSS selector or section to focus on (extract only matching content)
    #[serde(default)]
    pub focus: Option<String>,
    /// Skip cache and force fresh fetch
    #[serde(default)]
    pub no_cache: Option<bool>,
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

fn internal_err(msg: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(msg.to_string(), None)
}

#[allow(unexpected_cfgs)]
async fn fetch_with_strategy(
    strategy: &Strategy,
    request: FetchRequest,
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
        Strategy::Headless | Strategy::PublicApi => HttpSimple::new()?.fetch(request).await,
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

#[tool_router(server_handler)]
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

        let (cached, resolved) = {
            let storage = self.state.storage.lock().map_err(internal_err)?;
            let cached = if !no_cache {
                check_content_cache(&storage, &params.url)
            } else {
                None
            };
            let resolved = resolve_strategy(&storage, &params.url);
            (cached, resolved)
        };

        let markdown = if let Some(content) = cached {
            content
        } else {
            let request = FetchRequest {
                url: params.url.clone(),
                headers: HashMap::new(),
                timeout: Duration::from_secs(30),
                follow_redirects: true,
            };

            let start = std::time::Instant::now();
            let response = fetch_with_strategy(&resolved.strategy, request)
                .await
                .map_err(internal_err)?;
            let latency_ms = start.elapsed().as_millis() as u64;

            let html = String::from_utf8_lossy(&response.body);
            let extracted = extraction::extract(&html, &params.url, &response.content_type)
                .map_err(|e| internal_err(format!("extraction error: {e}")))?;

            let markdown = extracted
                .map(|e| e.text_content)
                .unwrap_or_else(|| html.into_owned());

            {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                store_and_record(
                    &storage,
                    &params.url,
                    &resolved.domain,
                    &resolved.strategy,
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

        let _ = &params.focus;

        Ok(CallToolResult::success(vec![Content::text(result)]))
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

        let ddg_url = format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencoding(&query)
        );

        let backend = HttpSimple::new().map_err(internal_err)?;
        let request = FetchRequest {
            url: ddg_url,
            headers: HashMap::new(),
            timeout: Duration::from_secs(15),
            follow_redirects: true,
        };

        let response = backend.fetch(request).await.map_err(internal_err)?;

        let html = String::from_utf8_lossy(&response.body);
        let results = parse_ddg_results(&html, max);

        if results.is_empty() {
            return Ok(CallToolResult::success(vec![Content::text(
                "No results found.".to_string(),
            )]));
        }

        if !fetch_content {
            let mut output = String::new();
            for (i, (title, url)) in results.iter().enumerate() {
                output.push_str(&format!("{}. {}\n   {}\n\n", i + 1, title, url));
            }
            return Ok(CallToolResult::success(vec![Content::text(output)]));
        }

        let fetch_limit = results.len().min(3);
        let mut output = String::new();

        for (i, (title, url)) in results.iter().take(fetch_limit).enumerate() {
            output.push_str(&format!("## {} - {}\n\n", i + 1, title));

            let (cached, resolved) = {
                let storage = self.state.storage.lock().map_err(internal_err)?;
                let cached = check_content_cache(&storage, url);
                let resolved = resolve_strategy(&storage, url);
                (cached, resolved)
            };

            if let Some(content) = cached {
                let preview = truncate_preview(&content, 2000);
                output.push_str(&preview);
                output.push_str("\n\n");
            } else {
                let req = FetchRequest {
                    url: url.clone(),
                    headers: HashMap::new(),
                    timeout: Duration::from_secs(30),
                    follow_redirects: true,
                };

                let start = std::time::Instant::now();
                match fetch_with_strategy(&resolved.strategy, req).await {
                    Ok(resp) => {
                        let latency_ms = start.elapsed().as_millis() as u64;
                        let body = String::from_utf8_lossy(&resp.body);
                        let extracted = extraction::extract(&body, url, &resp.content_type)
                            .map_err(|e| internal_err(format!("extraction error: {e}")))?;

                        let content = extracted
                            .map(|e| e.text_content)
                            .unwrap_or_else(|| body.into_owned());

                        {
                            let storage = self.state.storage.lock().map_err(internal_err)?;
                            store_and_record(
                                &storage,
                                url,
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
                        output.push_str(&format!("Failed to fetch: {e}\n\n"));
                    }
                }
            }
        }

        for (i, (title, url)) in results.iter().skip(fetch_limit).enumerate() {
            output.push_str(&format!("{}. {} — {}\n", fetch_limit + i + 1, title, url));
        }

        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

fn truncate_preview(content: &str, max_len: usize) -> String {
    if content.len() > max_len {
        format!("{} [...]", &content[..max_len])
    } else {
        content.to_string()
    }
}

fn urlencoding(s: &str) -> String {
    let mut encoded = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

fn parse_ddg_results(html: &str, max: usize) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < max {
        let result_start = match html[pos..].find("class=\"result__a\"") {
            Some(i) => pos + i,
            None => break,
        };

        let title = extract_tag_content(html, result_start);

        let href_start = match html[..result_start].rfind("href=\"") {
            Some(i) => i + 6,
            None => {
                pos = result_start + 16;
                continue;
            }
        };

        let href_end = match html[href_start..].find('"') {
            Some(i) => href_start + i,
            None => {
                pos = result_start + 16;
                continue;
            }
        };

        let href = html[href_start..href_end].to_string();
        let url = clean_ddg_url(&href);

        if url.starts_with("http") {
            results.push((title, url));
        }

        pos = result_start + 16;
    }

    results
}

fn extract_tag_content(html: &str, tag_pos: usize) -> String {
    let after_tag = match html[tag_pos..].find('>') {
        Some(i) => tag_pos + i + 1,
        None => return String::new(),
    };

    let end = match html[after_tag..].find('<') {
        Some(i) => after_tag + i,
        None => return html[after_tag..].to_string(),
    };

    html[after_tag..end]
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .trim()
        .to_string()
}

fn clean_ddg_url(url: &str) -> String {
    if let Some(start) = url.find("uddg=") {
        let encoded = &url[start + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        let encoded_portion = &encoded[..end];
        match percent_decode(encoded_portion) {
            Some(decoded) => return decoded,
            None => return encoded_portion.to_string(),
        }
    }
    url.strip_prefix("//")
        .map(|s| format!("https://{s}"))
        .unwrap_or_else(|| url.to_string())
}

fn percent_decode(s: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next()?.to_digit(16)? as u8;
            let lo = chars.next()?.to_digit(16)? as u8;
            result.push(char::from((hi << 4) | lo));
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    Some(result)
}

pub async fn serve_stdio(server: KaeloServer) -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting Kaelo MCP server");

    let service = server.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Serving error: {e:?}");
    })?;

    tracing::info!("Server connected, waiting for client messages...");
    service.waiting().await?;

    tracing::info!("Server shut down cleanly");
    Ok(())
}
