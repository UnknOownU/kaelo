# MCP Tools Reference

Complete reference for all Kaelo MCP tools. These tools are available when Kaelo is configured as an MCP server (stdio transport).

## Overview

| Tool | Purpose | Key Parameters |
|------|---------|----------------|
| `web_fetch` | Fetch a URL and return clean Markdown | `url`, `strategy`, `token_budget`, `focus`, `session` |
| `web_search` | Search the web via DuckDuckGo (with optional SearXNG fallback) | `query`, `max_results`, `fetch_content` |
| `fetch_urls` | Batch fetch up to 10 URLs in parallel | `urls`, `strategy`, `token_budget` |
| `ping` | Health check | none |
| `cache_status` | Show cache statistics | none |
| `cache_clear` | Clear all cached content | none |

---

## web_fetch

Fetches a URL and returns extracted Markdown content. This is the primary tool and the one used most frequently.

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `url` | string | Yes | — | The URL to fetch. HTTP URLs are auto-upgraded to HTTPS. |
| `strategy` | string | No | auto (route cache) | Fetch strategy. Leave empty to let Kaelo auto-detect based on cached per-domain results. Set explicitly only if you need to force a specific backend. |
| `token_budget` | u32 | No | 0 (unlimited) | Maximum token budget (approximate character count / 4). Content exceeding the budget is truncated with a `[... truncated to fit token budget]` suffix. |
| `focus` | string | No | — | CSS selector or section to focus on (extract only matching content). |
| `no_cache` | bool | No | false | Skip the content cache and force a fresh fetch. |
| `session` | string | No | — | Browser session ID. Pages in the same session share cookies and state. Sessions expire after 5 minutes of inactivity. Maximum 3 concurrent sessions. |

### Strategies

| Strategy | Description |
|----------|-------------|
| `HttpSimple` | Standard HTTP client. Used as the default fallback. |
| `TlsChrome` | TLS fingerprint impersonating Chrome. Requires the `tls-impersonation` feature; falls back to `HttpSimple` if not compiled in. |
| `TlsMobile` | TLS fingerprint impersonating a mobile browser. Same feature gate as `TlsChrome`. |
| `Headless` | Headless browser (full JavaScript rendering). Supports sessions. Use for pages that require JS execution. |
| `PublicApi` | Routes through a public API proxy. Internally uses `HttpSimple`. |

When no strategy is specified, Kaelo checks a route cache of per-domain success rates and latencies to pick the best strategy automatically.

### Behavior

1. If `no_cache` is false and a cached response exists, it is returned immediately.
2. On a cache miss, Kaelo fetches the URL using the resolved strategy.
3. The raw HTML is converted to Markdown via the extraction pipeline.
4. The result is stored in the content cache (1 hour TTL) and the route cache is updated with latency and success data.
5. If `token_budget` is set and content exceeds it, the result is truncated.

### Examples

Fetch a page with auto-detected strategy:

```json
{
  "name": "web_fetch",
  "arguments": {
    "url": "https://example.com/docs"
  }
}
```

Force headless rendering for a JS-heavy page:

```json
{
  "name": "web_fetch",
  "arguments": {
    "url": "https://spa-app.example.com/dashboard",
    "strategy": "Headless"
  }
}
```

Fetch with a token budget and cache bypass:

```json
{
  "name": "web_fetch",
  "arguments": {
    "url": "https://example.com/long-article",
    "token_budget": 2000,
    "no_cache": true
  }
}
```

### Error Handling

| Error | Cause |
|-------|-------|
| Internal error (fetch failed) | Network failure, DNS resolution failure, timeout (30s), or TLS error. The strategy and URL are logged. |
| Internal error (extraction error) | The fetched content could not be parsed or converted to Markdown. |
| Internal error (storage lock) | The internal storage mutex is poisoned. This indicates a serious runtime issue. |

---

## web_search

Searches the web and returns results. Uses DuckDuckGo by default. If the server is configured with a SearXNG instance and the SearXNG backend is selected, SearXNG is tried first with automatic fallback to DuckDuckGo on failure.

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `query` | string | Yes | — | Search query string. |
| `max_results` | u32 | No | 5 | Maximum number of results to return. Capped at 10. |
| `fetch_content` | bool | No | false | When true, fetches the top results' full content (up to 3) instead of just returning URLs and titles. Uses a search-then-fetch pipeline. |

### Behavior

Without `fetch_content` (default):

Returns a numbered list of results with titles and URLs.

```
1. Result Title
   https://example.com/page

2. Another Result
   https://example.org/another
```

With `fetch_content: true`:

Fetches full content for up to 3 top results. Each result section includes a heading with the result number and title, followed by the extracted content (truncated to 2000 characters). Remaining results beyond the fetch limit are listed as titles and URLs.

### Examples

Basic search:

```json
{
  "name": "web_search",
  "arguments": {
    "query": "Rust async HTTP client comparison"
  }
}
```

Search with content fetching:

```json
{
  "name": "web_search",
  "arguments": {
    "query": "Rust async HTTP client comparison",
    "fetch_content": true,
    "max_results": 3
  }
}
```

### Error Handling

| Error | Cause |
|-------|-------|
| Internal error (search failed) | DuckDuckGo API returned an error or was unreachable. If SearXNG was configured and failed, this error only appears if the DDG fallback also fails. |
| "No results found." | The query returned zero results. This is a successful response, not an error. |
| Internal error (extraction error) | Content fetching (`fetch_content: true`) succeeded but the page could not be converted to Markdown. |
| "Failed to fetch: ..." | Content fetching for a specific result URL failed. Other results are still returned. |

---

## fetch_urls

Fetches multiple URLs in parallel and returns their combined Markdown content, separated by `---` dividers.

### Parameters

| Parameter | Type | Required | Default | Description |
|-----------|------|----------|---------|-------------|
| `urls` | string[] | Yes | — | List of URLs to fetch. Minimum 1, maximum 10. |
| `strategy` | string | No | auto (route cache) | Fetch strategy applied to all URLs. Same strategy options as `web_fetch`. |
| `token_budget` | u32 | No | 0 (unlimited) | Maximum token budget per URL (not total). Applied individually to each result. |

### Behavior

1. Validates the URL list (must have at least 1, at most 10 entries).
2. Spawns a parallel task for each URL. Each task independently checks the cache, resolves the strategy, fetches, and extracts.
3. Results are collected in order and joined with `\n\n---\n\n` separators.
4. If an individual URL fails, an error string is included in that position (e.g., `[fetch error for https://example.com: timeout]`). Other results are not affected.

### Examples

Batch fetch documentation pages:

```json
{
  "name": "fetch_urls",
  "arguments": {
    "urls": [
      "https://docs.rs/serde/latest/serde/",
      "https://docs.rs/serde_json/latest/serde_json/",
      "https://docs.rs/tokio/latest/tokio/"
    ],
    "token_budget": 4000
  }
}
```

Force a specific strategy for all URLs:

```json
{
  "name": "fetch_urls",
  "arguments": {
    "urls": [
      "https://example.com/page1",
      "https://example.com/page2"
    ],
    "strategy": "Headless"
  }
}
```

### Error Handling

| Error | Cause |
|-------|-------|
| "At least one URL required" | The `urls` array is empty. |
| "Maximum 10 URLs per batch" | More than 10 URLs were provided. |
| `[fetch error for {url}: {message}]` | An individual URL failed to fetch (network error, timeout, etc.). Other URLs in the batch may still succeed. |
| `[extraction error for {url}: {message}]` | An individual URL was fetched but its content could not be converted to Markdown. |
| `[task error: {message}]` | A spawned task panicked or was cancelled. |
| `[error: {message}]` | Failed to acquire the storage lock for a URL's cache/strategy lookup. |

---

## ping

Health check tool. Returns "pong" if the server is responsive.

### Parameters

None.

### Example

```json
{
  "name": "ping",
  "arguments": {}
}
```

Returns:

```
pong
```

---

## cache_status

Shows statistics about the server's internal caches.

### Parameters

None.

### Example

```json
{
  "name": "cache_status",
  "arguments": {}
}
```

Returns:

```
Route cache: 42 entries
Content cache: 128 entries (1048576 bytes)
```

- **Route cache**: Stores per-domain strategy success rates and latencies. Used by `web_fetch` and `fetch_urls` for auto strategy selection.
- **Content cache**: Stores fetched Markdown content keyed by URL. 1 hour TTL.

### Error Handling

| Error | Cause |
|-------|-------|
| "Error acquiring storage lock: ..." | The internal storage mutex is poisoned. |

---

## cache_clear

Clears all cached content from both the route cache and the content cache.

### Parameters

None.

### Example

```json
{
  "name": "cache_clear",
  "arguments": {}
}
```

Returns:

```
Cache cleared: 42 routes, 128 content entries removed
```

### Error Handling

| Error | Cause |
|-------|-------|
| "Error acquiring storage lock: ..." | The internal storage mutex is poisoned. |
| "Error clearing content cache: ..." | The SQLite content cache table could not be cleared. |
| "Error clearing route cache: ..." | The SQLite domain_strategies table could not be cleared. |

---

## Shared Details

### Caching

All fetch tools (`web_fetch`, `fetch_urls`, and the `fetch_content` path in `web_search`) share a common caching layer:

- **Content cache**: Keyed by full URL. 1 hour TTL. Stores the extracted Markdown.
- **Route cache**: Keyed by domain. Records which strategy worked best (success rate, latency) for each domain.

The `no_cache` parameter on `web_fetch` bypasses both caches for that single request. `fetch_urls` and `web_search` (fetch_content path) always check the cache first.

### Token Budget

When `token_budget` is set to a value greater than 0, the output is truncated to approximately `token_budget * 4` characters. A `[... truncated to fit token budget]` suffix is appended to indicate truncation.

- On `web_fetch`, the budget applies to the single result.
- On `fetch_urls`, the budget applies per URL (not to the combined output).
- On `web_search` with `fetch_content`, each fetched result is truncated to 2000 characters (hardcoded, not configurable via `token_budget`).

### Search Backend Configuration

The search backend is configured at server startup:

- `search_backend: "ddg"` (default): Uses DuckDuckGo directly.
- `search_backend: "searxng"` with `searxng_url` set: Uses SearXNG first, falls back to DuckDuckGo if SearXNG returns empty results or errors.

This is not configurable per-request; it is a server-level setting.
