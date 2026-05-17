# Architecture

Kaelo is a Rust MCP server that fetches web content for AI agents. It picks the right fetch strategy per domain, caches what works, and returns clean Markdown within token budgets.

## Crate Structure

A single crate with a modular layout:

```
src/
├── main.rs              # CLI entrypoint (clap)
├── lib.rs               # Module declarations
├── config.rs            # Configuration (env vars, TOML, defaults)
├── types.rs             # Shared types (FetchRequest, FetchResponse, Strategy, FetchError)
├── error.rs             # Error types
├── update.rs            # Self-update from GitHub releases
├── extraction/          # Content extraction pipeline
├── fetch/               # FetchBackend trait + backends
├── mcp/                 # MCP server with tool definitions
├── router/              # Strategy selection and fallback chain
├── search/              # Web search backends
└── storage/             # SQLite (route cache, content cache, migrations)
```

The modules form a clean dependency chain: `main` → `mcp` → `fetch` → `core modules` (storage, extraction, router, etc.).

## Request Flow

Here's what happens when an agent calls `web_fetch`:

```
Agent (Claude, OpenCode, Cursor, etc.)
  │
  │ JSON-RPC over stdio
  ▼
┌──────────────────────────────────────────────────────────┐
│  KaeloServer (mcp module)                                 │
│                                                          │
│  1. Lock Storage                                         │
│  2. Check content cache (url_cache table)                │
│     ├─ HIT  → return cached Markdown                     │
│     └─ MISS → continue                                   │
│  3. Resolve strategy                                     │
│     ├─ Explicit (user set strategy param)? → use it      │
│     └─ Auto? → query route cache for domain             │
│        ├─ Known domain → use cached best strategy       │
│        └─ Unknown domain → default to HttpSimple         │
│  4. Fetch via selected backend                           │
│  5. Extract: raw HTML → Markdown via dom_smoothie        │
│  6. Store result in content cache + update route cache   │
│  7. Apply token budget truncation if needed              │
│  8. Return Markdown to agent                             │
└──────────────────────────────────────────────────────────┘
```

The `fetch_urls` tool runs the same pipeline per URL, but spawns each as a tokio task for parallel execution (up to 10 URLs).

## Router and Strategy Resolution

The router decides *how* to fetch a given URL. It lives in `src/router/`.

### Decision types

```rust
enum RouterDecision {
    Known { strategy: Strategy, score: f64 },  // cached, proven
    Unknown { strategy: Strategy },             // fall back to default
}
```

### Two resolution paths

**Fast path** (`resolve`): look up the domain in the route cache. If a strategy exists, return it with its computed score. No network calls.

**Probing path** (`resolve_with_probing`): for unknown domains, try each strategy in fallback order with a 5-second timeout per strategy. Cache the fastest successful one. This is slow (up to 20s for 4 strategies) but only happens on first visit.

### Scoring formula

```
score = success_rate * (1.0 / max(avg_latency_ms / 1000, 0.001))
```

High success rate and low latency yield the best score. The route cache stores all tried strategies per domain, sorted by score.

### Fallback chain

When a fetch fails, the router classifies the error and decides whether to try the next strategy:

```
ErrorClass          Response                Action
─────────          ────────                ──────
Transient           Timeout, network error   Try next backend
Blocking            403, 503                 Try TLS/headless
NotFound            404                      Stop, don't retry
RateLimited         429                      Stop, don't retry
```

Fallback order: `HttpSimple → TlsChrome → TlsMobile → Headless`

The `FallbackChain` struct tracks visited strategies and attempt count to avoid infinite loops.

### Seed domains

Kaelo ships with pre-seeded knowledge for common domains (defined in `seed_domains.rs`). On first run, these get inserted into the route cache so popular sites skip probing entirely:

| Domain | Strategy | Why |
|--------|----------|-----|
| reddit.com | PublicApi | JSON API avoids Cloudflare |
| medium.com | TlsChrome | Cloudflare challenge on articles |
| docs.rs | HttpSimple | Rust docs, no bot protection |
| notion.so | TlsChrome | Cloudflare, JS-rendered |
| developer.mozilla.org | HttpSimple | Open access |
| ... | ... | 30+ domains total |

## Route Cache

The route cache is Kaelo's central innovation. It's a SQLite table (`domain_strategies`) that remembers which fetch strategy works best for each domain.

### Schema

```sql
CREATE TABLE domain_strategies (
    domain TEXT NOT NULL,
    strategy TEXT NOT NULL,
    avg_latency_ms INTEGER NOT NULL DEFAULT 0,
    success_rate REAL NOT NULL DEFAULT 0.0,
    total_requests INTEGER NOT NULL DEFAULT 0,
    last_success_at TEXT,
    last_check_at TEXT NOT NULL,
    extra_headers TEXT,
    PRIMARY KEY (domain, strategy)
);
```

Multiple strategies can exist per domain (composite primary key). The router picks the one with the highest score.

### Update logic

After every fetch, `upsert_strategy` runs:

1. If the (domain, strategy) pair exists, it applies an exponential moving average to update `success_rate` and `avg_latency_ms`, and increments `total_requests`.
2. If it's a new pair, it inserts with the current fetch's metrics.
3. A cap of 10,000 entries (configurable via `KAELO_MAX_ROUTE_ENTRIES`) prevents unbounded growth. Oldest entries are pruned when the limit is hit.

### Domain clearing

The `clear_domain` method removes all strategies for a domain. Useful when a site changes its bot protection and old cached strategies are stale.

## Fetch Backends

All backends implement the `FetchBackend` trait from `src/fetch/`:

```rust
trait FetchBackend {
    fn fetch(&self, request: FetchRequest) -> impl Future<Output = Result<FetchResponse, FetchError>>;
    fn name(&self) -> Strategy;
    fn supports_probe(&self) -> bool;
}
```

### HttpSimple

Plain `reqwest` HTTP client. No TLS fingerprinting, no browser automation.

- Fastest backend (no overhead)
- Works for most public sites: documentation, blogs, APIs
- Handles 304 Not Modified for conditional requests
- Default strategy for unknown domains
- Always compiled (no feature gate)

### TlsChrome / TlsMobile

TLS fingerprint impersonation via the `wreq` crate. Emulates Chrome 131's TLS handshake.

- Feature-gated behind `tls-impersonation`
- Gets past Cloudflare and similar bot-detection services
- Both variants use the same `TlsImpersonation` struct; the strategy name determines which TLS profile wreq selects
- Falls back to HttpSimple when the feature is not compiled in
- Used for Cloudflare-protected sites (Medium, Discord, Notion, Stripe, etc.)

### Headless

Headless Chromium via `chromiumoxide`. The heaviest but most capable backend.

- Feature-gated behind `headless`
- Requires Chromium installed (`brew install --cask chromium`)
- Uses a global `BrowserPool` (via `OnceLock`) to reuse Chrome processes
- Randomized viewport sizes to avoid fingerprinting
- Injects JS to hide webdriver properties
- Waits for page content to render (MutationObserver with 8s timeout)
- Supports browser sessions: pages in the same session share cookies and state
- Max 3 concurrent sessions, 5-minute inactivity expiry
- Used for JS-heavy SPAs (Bloomberg, RSI Galactapedia, etc.)

### PublicApi

Native API clients for specific platforms. Converts a public URL into an API call that returns structured JSON.

| Platform | API |
|----------|-----|
| Reddit | `https://www.reddit.com/comments/{article_id}.json` |
| Hacker News | `https://hacker-news.firebaseio.com/v0/item/{id}.json` |
| YouTube | `https://www.youtube.com/oembed?url=...&format=json` |

Returns JSON that the extraction pipeline pretty-prints as Markdown. Much faster than scraping the HTML and completely bypasses bot detection.

## Content Extraction

The extraction pipeline converts raw HTTP responses into clean Markdown. It lives in `src/extraction/`.

### Pipeline stages

```
Raw response body
  │
  ▼
Content-type dispatch
  ├─ text/html      → dom_smoothie readability extraction
  ├─ application/json → pretty-print
  ├─ text/plain     → pass through
  └─ binary/*       → "[Unsupported content type]" placeholder
  │
  ▼
HTML → Readability (dom_smoothie)
  ├─ Strip nav, header, footer, sidebar, ads
  ├─ Extract main content block
  └─ Convert to Markdown
  │
  ▼
Boilerplate detection (optional)
  ├─ Hash text blocks per domain
  ├─ Blocks seen on 3+ pages = boilerplate → strip
  └─ In-memory only, resets per session
  │
  ▼
Section scoring (when `focus` param is set)
  ├─ Parse Markdown into sections by headings
  ├─ Score each section by keyword overlap with focus terms
  ├─ Penalize structural boilerplate headings (seen on 50%+ of domain pages)
  └─ Return sections sorted by relevance
  │
  ▼
Token budget enforcement
  ├─ 1 token ≈ 4 characters
  ├─ If over budget: smart truncation at heading boundaries
  ├─ Prepends Table of Contents from all headings
  └─ Appends "[... truncated ...]" marker
```

### Key extraction modules

- **`mod.rs`**: entry point. Dispatches by content type, runs readability, returns `ExtractedContent`.
- **`boilerplate.rs`**: `BoilerplateTracker` hashes text blocks per domain. Blocks appearing on 3+ pages get flagged as boilerplate and stripped. Max block size: 2000 chars (larger blocks are assumed to be unique content).
- **`scoring.rs`**: `score_sections()` parses Markdown by headings, tokenizes focus terms, and computes relevance per section.
- **`structural.rs`**: `StructuralTracker` learns per-domain heading patterns over time. Sections with headings that appear on 50%+ of a domain's pages get penalized (they're likely nav/footer repeated across pages).
- **`token_estimator.rs`**: `truncate_smart()` tries to break at heading boundaries. Falls back to flat truncation if the document has no headings. Always prepends a TOC when truncation occurs.

## Cache Layer

Two separate caches, both backed by the same SQLite database.

### Content cache

Stores extracted Markdown content keyed by URL.

```sql
CREATE TABLE url_cache (
    url TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    compressed_content BLOB,
    content_type TEXT,
    original_size INTEGER,
    created_at TEXT NOT NULL,
    last_accessed_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    etag TEXT,
    last_modified TEXT
);
```

Key properties:

- **gzip compression**: content is compressed before storage via `flate2`. Decompressed on read.
- **SHA-256 content hashing**: prevents storing duplicate content.
- **TTL-based expiry**: default 1 hour. Expired entries return `None` on `get()`.
- **Max entry size**: 100 KB uncompressed. Larger content is not cached.
- **Conditional request support**: ETag and Last-Modified columns (added in V002 migration) enable future conditional re-fetches.
- **Cache bypass**: `no_cache` parameter on `web_fetch` skips the cache entirely.

### Route cache

Described above in the Router section. Stores per-domain strategy performance data.

### Storage

Both caches share a single `Storage` struct wrapping a `rusqlite::Connection`. The database uses WAL journal mode for better concurrent read performance. Migrations are versioned (V001, V002) and run automatically on open.

Default database path: `~/.config/kaelo/kaelo.db`. In-memory mode (`:memory:`) is used for tests.

## MCP Server

The MCP integration follows the [Model Context Protocol](https://modelcontextprotocol.io/) spec (version 2024-11-05).

### Transport

stdio. The agent spawns `kaelo` as a subprocess and communicates via JSON-RPC over stdin/stdout. No network port needed.

### State management

```rust
struct KaeloState {
    storage: Mutex<Storage>,
    searxng_url: Option<String>,
    search_backend: Option<String>,
}
```

Shared via `Arc<KaeloState>`. Storage is behind a `Mutex` because rusqlite connections are not `Send`.

### Tool definitions

| Tool | Description | Key params |
|------|-------------|------------|
| `web_fetch` | Fetch a single URL, return Markdown | `url`, `strategy`, `token_budget`, `focus`, `no_cache`, `session` |
| `web_search` | Search the web, optionally fetch top result | `query`, `max_results`, `fetch_content` |
| `fetch_urls` | Batch fetch up to 10 URLs in parallel | `urls`, `strategy`, `token_budget` |
| `ping` | Health check | none |

Tools are defined using the `rmcp` crate's `#[tool]` attribute macro. The `#[tool_router]` attribute on the `impl KaeloServer` block generates the MCP tool routing.

### Error handling

Fetch errors are classified and returned as MCP error data. The fallback chain handles retries internally before surfacing an error to the agent.

## Search

Two pluggable search backends behind a `SearchBackend` trait:

```rust
trait SearchBackend: Send + Sync {
    fn search(&self, query: &str, max_results: usize) -> impl Future<Output = Result<Vec<SearchResult>>>;
}
```

### DuckDuckGo (default)

Scrapes the HTML version of DuckDuckGo search results. Parses `class="result__a"` elements to extract titles and URLs. No API key needed.

### SearXNG (optional)

Queries a self-hosted SearXNG instance via its JSON API (`/search?format=json`). Configured via `searxng_url` config or `KAELO_SEARXNG_URL` env var. Returns richer results with descriptions.

### Search-then-fetch

The `web_search` tool supports `fetch_content: true`, which automatically fetches the top search result's full content in a single call. This combines search and fetch into one round-trip.

## Configuration

Config is loaded from `~/.config/kaelo/config.toml` with environment variable overrides.

Key settings:

| Setting | Env var | Default |
|---------|---------|---------|
| Database path | `KAELO_DB_PATH` | `~/.config/kaelo/kaelo.db` |
| Cache enabled | `KAELO_CACHE_ENABLED` | `true` |
| Cache max size | `KAELO_CACHE_MAX_SIZE` | `50MB` |
| Cache TTL | `KAELO_CACHE_TTL` | `3600s` |
| Default strategy | `KAELO_DEFAULT_STRATEGY` | auto |
| Search backend | `KAELO_SEARCH_BACKEND` | `ddg` |
| SearXNG URL | `KAELO_SEARXNG_URL` | none |
| Per-domain auth | `KAELO_AUTH_{DOMAIN}` | none |

Per-domain auth supports Bearer tokens and Basic auth via env vars like `KAELO_AUTH_GITHUB_COM=Bearer:ghp_xxx`.

## Feature Flags

Heavy dependencies are feature-gated:

| Feature | Default | What it enables |
|---------|---------|-----------------|
| `headless` | yes | chromiumoxide, headless browser backend |
| `tls-impersonation` | yes | wreq, TLS fingerprint impersonation |

Disable with `cargo build --no-default-features` to get a minimal binary with only HttpSimple and PublicApi backends.

## Testing

- **Unit tests per crate**: each crate has its own test suite covering core logic (router, cache, extraction, search parsing).
- **Integration tests for backends**: fetch backends have tests that verify client construction and (where possible) actual HTTP behavior.
- **252 tests total** as of the latest CI run.
- In-memory SQLite (`:memory:`) is used for all storage tests, so tests run without filesystem side effects.
