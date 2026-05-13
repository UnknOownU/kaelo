# Kaelo — Product Requirements Document

> Intelligent web fetching MCP server for AI agents. Rust-native, local-first, token-aware.

## 1. Executive Summary

### Problem

AI coding agents (OpenCode, Claude Code, Cursor, etc.) need to access web content constantly — documentation, GitHub repos, Stack Overflow, APIs. But websites increasingly block automated access: WAFs, CAPTCHAs, bot detection, paywalls, JavaScript-only SPAs.

Current solutions are either:
- **Paid cloud services** (Exa, Tavily, Firecrawl) — expensive, dependent on external infrastructure
- **Brute-force local scrapers** (webclaw, insane-search) — they work, but re-learn the same lessons every single request
- **Fragile single-strategy tools** — break when sites change protections

### Solution

**Kaelo** is a local MCP server written in Rust that exposes web fetching and search tools to AI agents. Its core innovation: **an intelligent router that learns from every request**, remembering which strategy works for which domain — and getting faster over time.

### Key Differentiator

> **Other tools optimize extraction. Kaelo optimizes what the AI consumes.**

| Capability | webclaw | insane-search | Firecrawl | **Kaelo** |
|---|---|---|---|---|
| Local, no cloud | Mostly | Yes | No | **Yes** |
| Rust-native | Yes | No (Python) | No | **Yes** |
| MCP server | Yes | No (plugin) | No | **Yes** |
| Intelligent routing | No | No | No | **Yes** |
| Learning cache | No | No | No | **Yes** |
| Token-aware | No | No | No | **Yes** |
| Free, no API key | Mostly | Yes | No | **Yes** |
| License permissive | AGPL-3.0 | MIT | Commercial | **MIT** |

---

## 2. Target Users

### Primary

- **AI coding agents** — OpenCode, Claude Code, Cursor, Windsurf, Copilot, Codex
- **Agent orchestration frameworks** — OhMyOpenAgent, Kasetto, OpenHands
- **Developers building AI-powered tools** — RAG pipelines, research agents, documentation bots

### Secondary

- **Security researchers** — legitimate web content access through anti-bot protections
- **Data engineers** — lightweight local web content extraction
- **Privacy-conscious users** — want web content without sending data to cloud services

---

## 3. Architecture Overview

### Design Philosophy

Kaelo does NOT re-implement what already exists. It orchestrates:

- **Lightweight backends** for execution (reqwest, chromiumoxide, readability)
- **Intelligence layer** for routing, caching, and token optimization
- **Learning store** for persistent knowledge accumulation

### High-Level Architecture

```
┌──────────────────────────────────────────────────────┐
│                  Kaelo MCP Server                     │
│                   (Rust binary)                       │
│                                                       │
│  MCP Interface (stdio transport, rmcp crate)         │
│  ├─ web_fetch(url, options) → Markdown                │
│  ├─ web_search(query) → URLs + snippets               │
│  └─ web_extract(url, query) → Targeted content        │
│                                                       │
│  ┌──────────────────────────────────────────┐        │
│  │  INTELLIGENT ROUTER                       │        │
│  │                                           │        │
│  │  Input: URL                               │        │
│  │  1. Domain lookup in route cache           │        │
│  │  2. Known? → Use learned strategy          │        │
│  │  3. Unknown? → Probe & discover            │        │
│  │  4. Store result in route cache            │        │
│  │                                           │        │
│  │  Learns: domain → {strategy, latency,      │        │
│  │           success_rate, last_check}        │        │
│  └──────────┬───────────────────────────────┘        │
│             │                                         │
│  ┌──────────▼───────────────────────────────┐        │
│  │  BACKENDS (execution layer)                │        │
│  │                                           │        │
│  │  A. HTTP Simple (reqwest)                 │        │
│  │     For: most documentation sites, APIs   │        │
│  │     Speed: 100-500ms                      │        │
│  │                                           │        │
│  │  B. TLS Impersonation (utls/boring)       │        │
│  │     For: Cloudflare-protected sites       │        │
│  │     Speed: 500ms-2s                       │        │
│  │                                           │        │
│  │  C. Headless Browser (chromiumoxide)      │        │
│  │     For: JS-heavy SPAs, complex WAFs      │        │
│  │     Speed: 2-5s                           │        │
│  │                                           │        │
│  │  D. Public APIs (.json, RSS, etc.)        │        │
│  │     For: Reddit, YouTube, HN, arXiv       │        │
│  │     Speed: 100-300ms                      │        │
│  │                                           │        │
│  │  E. Search (SearXNG local)                │        │
│  │     For: web search queries               │        │
│  │     Speed: 500ms-2s                       │        │
│  └───────────────────────────────────────────┘        │
│                                                       │
│  ┌───────────────────────────────────────────┐       │
│  │  TOKEN-AWARE LAYER                         │       │
│  │                                            │       │
│  │  - Content deduplication (in-session)      │       │
│  │  - Token budget enforcement                │       │
│  │  - Targeted extraction (content-type aware)│       │
│  │  - Content compression for cache           │       │
│  └────────────────────────────────────────────┘       │
│                                                       │
│  ┌───────────────────────────────────────────┐       │
│  │  STORAGE (SQLite)                          │       │
│  │                                            │       │
│  │  Route cache: domain → strategy (~5 MB)   │       │
│  │  Content cache: hash → gzip blob (~12 MB) │       │
│  │  Configurable, bounded, auto-evicting      │       │
│  └────────────────────────────────────────────┘       │
└──────────────────────────────────────────────────────┘
```

---

## 4. Core Features

### 4.1 Intelligent Router

The heart of Kaelo. Every URL request passes through the router.

**Behavior:**

1. **Known domain** → Look up the route cache. Among all stored strategies for this domain, pick the one with the highest success rate. Skip probing.
2. **Unknown domain** → Execute a lightweight probe (HTTP GET with `Range: bytes=0-1024` header). Many sites reject HEAD requests, so a ranged GET is more reliable. Based on the response (status, headers, WAF fingerprints), select a strategy. If it fails, escalate to the next strategy.
3. **Store results** → After each request, update the route cache: strategy used, latency, success/failure. Multiple strategies can be stored per domain, each with their own stats.

**Route Cache Schema:**

```
Table: domain_strategies
- domain (TEXT)              -- e.g. "reddit.com"
- strategy (TEXT)            -- "http_simple", "tls_chrome", "tls_mobile", "headless", "public_api"
- avg_latency_ms (INTEGER)
- success_rate (REAL)        -- 0.0 to 1.0
- total_requests (INTEGER)
- last_success_at (TIMESTAMP)
- last_check_at (TIMESTAMP)
- extra_headers (TEXT)       -- JSON: custom headers that worked
PRIMARY KEY: (domain, strategy)   -- Composite key: multiple strategies per domain
```

**Strategy Selection:** When multiple strategies exist for a domain, Kaelo selects the one with the best balance of `success_rate` and `avg_latency_ms`. If the top strategy fails, it falls back to the next best. All attempts update the stats.

**Eviction:** Domains not accessed in 30 days are pruned. Individual strategies with `success_rate < 0.2` after 10+ attempts are removed. Manual clear available.

**Cold Start:** On first launch, Kaelo has zero knowledge. It builds its route cache organically from actual requests. No hardcoded domain lists.

### 4.2 Token-Aware Fetching

Kaelo is aware that its output is consumed by an LLM with limited context windows and token costs.

**Token Budget:**

The agent can specify a `token_budget` parameter. Kaelo will:
- Estimate token count of extracted content
- If over budget: truncate intelligently (keep headings, first paragraphs, code blocks)
- Return metadata: `{content, tokens_used, was_truncated, original_size}`

**Content Deduplication (in-session):**

When an agent fetches multiple URLs in the same session, Kaelo tracks content hashes. If two URLs return substantially similar content, Kaelo flags the duplicate and returns only the canonical version.

**Targeted Extraction:**

The agent can pass a `query` or `focus` parameter. Kaelo uses this to prioritize extraction:
- If fetching a documentation page with focus="authentication" → extract sections about auth, skip the rest
- If fetching a GitHub repo with focus="installation" → prioritize README installation section

**How it works (section-based heuristic):**
1. Parse the HTML into a section tree (headings → content blocks)
2. Score each section based on keyword overlap with the focus parameter
3. Return sections sorted by relevance score, within the token budget
4. If no focus is specified, return the full extracted content (standard behavior)

This is a heuristic approach (not semantic/embedding-based) to keep it fast and dependency-free. Post-MVP could add semantic ranking via local embeddings.

### 4.3 Content Cache

**Purpose:** Avoid re-fetching unchanged content.

**How it works:**

1. Fetch content using the optimal strategy (from route cache)
2. Extract and clean to Markdown
3. Compute SHA-256 hash
4. Compare with stored hash for this URL
5. If unchanged → signal to agent, return cached content
6. If changed → update cache, return new content with "changed: true" flag

**Cache Configuration:**

```
Cache settings (configurable via environment variables with `KAELO_` prefix, or TOML config file):
```
# Environment variables
KAELO_CACHE_ENABLED=true         # Master switch
KAELO_CACHE_MAX_SIZE=52428800    # 50 MB hard limit (in bytes)
KAELO_CACHE_MAX_ENTRY=102400     # Don't cache entries > 100 KB (in bytes)
KAELO_CACHE_TTL=3600             # Default TTL: 1 hour (in seconds)
KAELO_CACHE_COMPRESSION=gzip     # Compress stored content
KAELO_CACHE_EVICTION=lru         # Evict least recently used when full
```
```

**Actual disk usage with gzip compression:**

| Scenario | Raw content | Compressed | 500 pages | Calculation basis |
|---|---|---|---|---|
| Average article | ~100 KB | ~25 KB | ~12 MB | Typical gzip ratio: 70-80% reduction on HTML/text |

Note: Compression ratio varies by content type. Technical documentation compresses better (~80%) than prose-heavy articles (~70%). These are conservative estimates.

**Automatic eviction:**
- LRU when cache exceeds max_size
- TTL expiry on every cache read
- Startup cleanup of expired entries

**Manual cache management:**

```
kaelo cache status           -- Show size, entries, top domains
kaelo cache clear            -- Clear everything
kaelo cache clear --domain X -- Clear specific domain
kaelo cache clear --older-than 24h  -- Clear old entries
kaelo cache clear --strategies      -- Reset route cache (keep content cache)
```

**Cache paranoia levels:**

| Level | KAELO_CACHE_MAX_SIZE | KAELO_CACHE_ENABLED | Max disk usage | Notes |
|---|---|---|---|---|
| Parano | — | false | ~5 MB | Route cache only, no content cache |
| Light | 10485760 (10 MB) | true | ~3 MB | Short TTL recommended |
| Normal | 52428800 (50 MB) | true | ~12 MB | Default, good balance |
| Heavy | 209715200 (200 MB) | true | ~50 MB | Long-running sessions |

### 4.4 Search

Kaelo provides web search via a local SearXNG instance or direct search engine queries.

**Behavior:**
- If SearXNG is available locally → use it (zero API key, zero cost)
- If not → fallback to DuckDuckGo Lite HTML parsing (no API key needed)
- Results returned as structured data: title, URL, snippet, source

**Search + Fetch pipeline:**

When an agent uses `web_search`, Kaelo can optionally fetch the top results' content in parallel, returning both the search results and the full extracted content — saving the agent multiple round-trips.

### 4.5 MCP Interface

Kaelo exposes these MCP tools via stdio transport:

| Tool | Description | Key Parameters |
|---|---|---|
| `web_fetch` | Fetch a URL and return clean Markdown | url, token_budget, focus, no_cache |
| `web_search` | Search the web, return results | query, max_results, fetch_content |
| `web_extract` | Fetch and extract targeted content | url, query, token_budget |
| `cache_status` | Show cache statistics | — |
| `cache_clear` | Clear cache entries | domain, older_than, strategies |

**Integration with OpenCode:**

```jsonc
// opencode.json
{
  "mcp": {
    "kaelo": {
      "type": "local",
      "command": ["kaelo"],
      "enabled": true
    }
  }
}
```

**Integration with other MCP clients:** Kaelo works with any MCP-compatible client (Claude Desktop, Cursor, Windsurf, etc.) via stdio transport.

---

## 5. Competitive Landscape

### Direct Competitors

| Tool | Stars | Language | License | Key Gap |
|---|---|---|---|---|
| **webclaw** | 1,136 | Rust | AGPL-3.0 | No intelligent routing, no learning, cloud fallback for hard sites |
| **crw** | 83 | Rust | AGPL-3.0 | No learning, 12-crate monolith, AGPL |
| **insane-search** | 640 | Python | MIT | No MCP server, Claude Code only, no memory |
| **Firecrawl** | — | TypeScript | Commercial | Paid ($19-$499/mo), not local |
| **Argus** | — | TypeScript | — | Not Rust, no MCP |
| **AgentSearch** | — | Python | — | No TLS spoofing, not Rust |

### Kaelo's Positioning

```
                    LOCAL                              CLOUD
                      │                                  │
           webclaw    │                        Firecrawl  │
           crw        │                        Tavily     │
                      │                        Exa        │
                      │                                  │
   DUMB ROUTING       │              DUMB ROUTING         │
   ───────────────────┼────────────────────────────────── │
   INTELLIGENT         │                                  │
   ROUTING             │                                  │
                      │                                  │
               ★ Kaelo ★ │                                  │
                      │                                  │
```

Kaelo is the **only tool in the "local + intelligent" quadrant.**

### What Kaelo Does NOT Compete With

- **Exa/Tavily** — These are search engines with their own index. Kaelo is a fetcher/router, not an index.
- **webclaw** — Kaelo could use webclaw as a backend for complex sites. They're complementary, not competing.
- **SearXNG** — Kaelo uses SearXNG for search. Not competing.

---

## 6. Technical Constraints

### Language & Runtime

- **Language:** Rust (for performance, single binary, zero runtime)
- **Async runtime:** tokio
- **MCP SDK:** rmcp (official Rust SDK from modelcontextprotocol/rust-sdk)
- **Minimum Rust version:** 1.75+

### Dependencies (principles)

- Minimize external dependencies
- No runtime requiring Node.js, Python, or Bun
- Single static binary for macOS (Apple Silicon + Intel), Linux (x86_64 + ARM64)
- Optional: Windows support

### Storage

- **SQLite** via rusqlite for all persistent storage
- Single database file at `~/.config/kaelo/kaelo.db`
- Two tables: `domain_strategies` (route cache), `url_cache` (content cache)
- All content cache entries gzip-compressed

### External Services

- **Fetch operations** work without any external service — just Kaelo, the backends, and the target URL
- **Search operations** require internet access. Search backends:
  - **SearXNG** (preferred): Self-hosted, zero API key, full privacy. User must install separately.
  - **DuckDuckGo HTML** (fallback): No API key needed, but HTML parsing is fragile and subject to rate limiting
- **No API keys needed** for core fetch and search functionality
- **Optional:** User can configure API keys for enhanced search (Brave Search API, etc.)
- **Headless browser** requires a Chromium-based browser installed on the system. If absent, Kaelo degrades gracefully to HTTP + TLS backends only.

---

## 7. Non-Goals

These are explicitly OUT of scope:

- **Building a search engine index** — Kaelo fetches and searches, it does not index
- **Replacing webclaw/crw** — Kaelo is a different category of tool (router, not scraper)
- **Headless browser management** — Kaelo uses chromiumoxide but doesn't manage Chrome installs
- **CAPTCHA solving** — Kaelo works around CAPTCHAs, it doesn't solve them
- **Proxy/VPN management** — Out of scope for v1
- **Multi-user/server mode** — Kaelo is a single-user local tool
- **Web dashboard/UI** — CLI and MCP only

---

## 8. Success Metrics

### Performance

| Metric | Target |
|---|---|
| First visit to unknown domain | < 10s (including probing) |
| Repeated visit to known domain | < 2x optimal backend latency |
| Cache hit rate (after warm-up) | > 70% for frequent domains |
| Route cache lookup | < 1ms |
| Content cache hit return | < 10ms |

### Quality

| Metric | Target |
|---|---|
| Content extraction accuracy | > 90% (readable, relevant Markdown) |
| Token reduction vs raw fetch | > 50% on average |
| False positive route cache entries | < 5% |

### Resource Usage

| Metric | Target |
|---|---|
| Binary size | < 15 MB |
| Memory (idle) | < 20 MB |
| Memory (peak, during headless fetch) | < 200 MB |
| Disk (route cache, 10K domains) | < 5 MB |
| Disk (content cache, default config) | < 12 MB |
| Startup time | < 500ms |

---

## 9. MVP Scope (v0.1)

### Must Have

- [ ] MCP server with stdio transport (rmcp)
- [ ] `web_fetch` tool — fetch URL, return Markdown
- [ ] Route cache (SQLite) — domain → strategy
- [ ] 3 backends: HTTP simple, TLS impersonation, headless browser
- [ ] Intelligent router with probe & learn
- [ ] Basic HTML → Markdown extraction (readability)
- [ ] gzip content cache with LRU eviction
- [ ] CLI: `kaelo cache status`, `kaelo cache clear`
- [ ] Config via environment variables
- [ ] OpenCode integration (opencode.json config)

### Nice to Have (post-MVP)

- [ ] `web_search` tool (SearXNG or DuckDuckGo)
- [ ] `web_extract` tool with targeted extraction
- [ ] Token budget parameter
- [ ] Content deduplication (in-session)
- [ ] Content diffing (hash comparison)
- [ ] Search + fetch pipeline
- [ ] Config file (TOML)
- [ ] `kaelo cache clear --domain X`
- [ ] Homebrew formula (`brew install kaelo`)
- [ ] Community route cache sharing (export/import)

---

## 10. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| webclaw adds intelligent routing | Medium | High | First-mover advantage, focus on token-aware features they don't have |
| Sites change protections frequently | High | Medium | Route cache auto-invalidates after failures, re-probes automatically |
| Headless browser is heavy dependency | Medium | Medium | Make it optional; HTTP + TLS cover 80% of cases |
| AGPL competitors get all the attention | Low | Medium | MIT license is a genuine advantage for integration |
| Token optimization is hard to measure | Medium | Low | Start simple (truncation), iterate based on real usage |

---

## 11. Naming & Branding

**Name:** Kaelo

**Origin:** Invented name. Evokes "caelum" (sky/heaven in Latin) — the entity that sees and traverses everything from above.

**Pronunciation:** kaé-lo (2 syllables)

**Package:** `cargo install kaelo`

**MCP registration:** `"kaelo"` in client config

**Logo concept:** A stylized eye or celestial figure — something that conveys "seeing through barriers."

**Repository:** github.com/[user]/kaelo

---

## 12. Open Questions

These need to be resolved before implementation:

1. **Headless browser strategy** — Bundle Chromium or require system Chrome? Trade-off: binary size vs zero-config.
2. **SearXNG integration** — Bundle it, require it, or just use DuckDuckGo HTML?
3. **Community route cache** — Should users be able to export/import their learned strategies? This could create a "community knowledge base" of which strategies work for which domains.
4. **Rate limiting** — Should Kaelo self-impose rate limits per domain to avoid IP bans?
5. **Multi-agent contention** — What happens when multiple agents use Kaelo simultaneously? SQLite locking?
6. **Content cache granularity** — Cache per URL, or per URL + query params?
7. **Route cache confidence** — How many successful fetches before a route is "trusted"? How to handle strategy degradation over time?

---

*Document version: 1.1 — Oracle-reviewed, fixes applied on 2026-05-13*
