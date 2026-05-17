# Kaelo

> Intelligent web fetching for AI agents. Local-first, token-aware, learns as it goes.

[![CI](https://github.com/HachemiH/kaelo/actions/workflows/ci.yml/badge.svg)](https://github.com/HachemiH/kaelo/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.76%2B-orange.svg)](https://www.rust-lang.org/)
[![MCP](https://img.shields.io/badge/MCP-2024--11--05-purple.svg)](https://modelcontextprotocol.io/)
[![Version](https://img.shields.io/badge/version-1.0.0-green.svg)](https://github.com/HachemiH/kaelo/releases)

**[Installation](#quick-start)** · **[Configuration](#configuration)** · **[Architecture](#how-it-works)** · **[MCP Tools](#mcp-tools)** · **[Contributing](#contributing)**

---

## Demo

<!-- demo placeholder -->

## What is Kaelo?

Kaelo is a Rust MCP server that gives AI agents (OpenCode, Claude Code, Cursor, etc.) reliable access to web content, even when sites try to block them.

Unlike existing tools that brute-force every request with the same fallback chain, **Kaelo learns which strategy works for which domain** and gets faster over time.

```
First visit to reddit.com:   8.1s  (probing strategies)
Second visit to reddit.com:  1.5s  (uses known-good strategy)
100th visit to reddit.com:   1.5s  (same, every time)
```

## Why Kaelo?

| Problem | How Kaelo solves it |
|---|---|
| Sites block AI agents | Multi-strategy fetching: HTTP → TLS impersonation → headless browser |
| Every request re-learns from scratch | Route cache remembers what works per domain |
| Agents waste tokens on bloated content | Token-aware extraction: budgets, dedup, targeted extraction |
| Existing tools are AGPL or paid | MIT licensed, local-first, zero API keys for fetch |

## How it works

```
Agent requests URL
       │
       ▼
┌──────────────┐     ┌─────────────────────────────────┐
│   Router     │────▶│  Route Cache (SQLite)           │
│              │     │  reddit.com → TLS mobile (1.5s) │
│              │     │  github.com → HTTP simple (0.3s)│
│              │     │  bloomberg.com → headless (4.8s)│
└──────┬───────┘     └─────────────────────────────────┘
       │
       │ Known? → Use cached strategy
       │ Unknown? → Probe & learn
       │
       ▼
┌──────────────┐
│   Backends   │
│  ├ HTTP      │──▶ Most sites
│  ├ TLS spoof │──▶ Cloudflare-protected sites
│  ├ Headless  │──▶ JS-heavy SPAs
│  └ Public API│──▶ Reddit, HN, YouTube, etc.
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ Token-Aware  │──▶ Budget, dedup, targeted extraction
│    Layer     │
└──────┬───────┘
       │
       ▼
    Clean Markdown to the agent
```

## Quick Start

**From source:**

```bash
git clone https://github.com/HachemiH/kaelo.git
cd kaelo
cargo install --path .
```

**One-liner:**

```bash
curl -fsSL https://raw.githubusercontent.com/HachemiH/kaelo/main/install.sh | bash
```

**Homebrew:**

```bash
brew install --build-from-source packaging/homebrew/kaelo.rb
```

**Optional** — for JS-heavy SPA sites (RSI Galactapedia, Bloomberg, etc.):

```bash
brew install --cask chromium
xattr -cr /Applications/Chromium.app  # macOS Gatekeeper fix
```

Verify it works:

```bash
kaelo prove-it
```

## MCP Client Setup

Kaelo runs as an MCP server over stdio. Add it to your agent's config and it auto-starts with your session.

**OpenCode** (`opencode.json`):

```jsonc
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

**Claude Code** (`.claude/settings.json`):

```jsonc
{
  "mcpServers": {
    "kaelo": {
      "command": "kaelo"
    }
  }
}
```

**Cursor** (`.cursor/mcp.json`):

```jsonc
{
  "mcpServers": {
    "kaelo": {
      "command": "kaelo",
      "args": ["serve"]
    }
  }
}
```

## MCP Tools

| Tool | Description | Parameters |
|---|---|---|
| `web_fetch` | Fetch a URL and return clean Markdown | `url`, `strategy`, `token_budget`, `focus`, `no_cache`, `session` |
| `web_search` | Search the web, optionally fetch top results | `query`, `max_results`, `fetch_content` |
| `fetch_urls` | Batch fetch up to 10 URLs in one call | `urls`, `strategy`, `token_budget` |
| `ping` | Health check, returns pong | — |

### Tool details

**`web_fetch`** — the primary tool. Fetches a URL and returns extracted Markdown content. Supports CSS selector focusing (`focus`), token budget limits, browser sessions for cookie-aware navigation, and cache bypass.

**`web_search`** — searches DuckDuckGo (or a self-hosted SearXNG instance) and returns results. Set `fetch_content: true` to automatically fetch the top result's full content in a single call.

**`fetch_urls`** — parallel batch fetch. Pass up to 10 URLs and get combined Markdown output separated by `---`. Each URL can have its own token budget.

## Fetch Strategies

Kaelo auto-selects the best strategy per domain using its route cache. You can also force a specific strategy via the `strategy` parameter.

| Strategy | Backend | When to use |
|---|---|---|
| `HttpSimple` | reqwest | Static sites, APIs, most pages. The default for unknown domains. |
| `TlsChrome` | wreq (TLS impersonation) | Cloudflare-protected sites, bot-detection pages. |
| `TlsMobile` | wreq (mobile TLS) | Sites that block desktop bots but allow mobile traffic. |
| `Headless` | chromiumoxide | JS-heavy SPAs (React, Next.js, Vue, Blazor). Requires Chromium. |
| `PublicApi` | native HTTP | Reddit, Hacker News, YouTube. Uses native APIs for structured data. |

Leave `strategy` empty to let Kaelo auto-detect. It probes on the first visit and caches the result for subsequent requests.

## Configuration

### Config file

`~/.config/kaelo/config.toml` (created automatically on first run):

```toml
cache_enabled = true
cache_max_size = 52428800   # 50 MB
cache_max_entry = 102400    # 100 KB per entry
cache_ttl = 3600            # 1 hour
cache_compression = "gzip"
db_path = "~/.config/kaelo/kaelo.db"
log_level = "info"

# Optional: self-hosted search backend
# searxng_url = "http://localhost:8888"
# search_backend = "searxng"

# Optional: override default strategy for all requests
# default_strategy = "HttpSimple"
```

### Environment variables

Environment variables override config file values:

```
KAELO_CACHE_ENABLED=true          # Master cache switch
KAELO_CACHE_MAX_SIZE=52428800     # 50 MB hard limit
KAELO_CACHE_MAX_ENTRY=102400      # 100 KB max per cached entry
KAELO_CACHE_TTL=3600              # 1 hour default TTL
KAELO_CACHE_COMPRESSION=gzip      # Compress cached content
KAELO_DB_PATH=~/.config/kaelo/kaelo.db  # SQLite database path
KAELO_SEARXNG_URL=http://...      # SearXNG instance URL
KAELO_SEARCH_BACKEND=searxng      # "duckduckgo" (default) or "searxng"
KAELO_DEFAULT_STRATEGY=HttpSimple # Override auto-detection for all requests
KAELO_LOG_LEVEL=debug             # log level
```

### Per-domain auth

For sites that require authentication, set environment variables with the domain:

```bash
# Bearer token for a private GitHub repo
KAELO_AUTH_GITHUB_COM="Bearer: ghp_xxxxxxxxxxxx"

# Basic auth for a staging site
KAELO_AUTH_STAGING_EXAMPLE_COM="Basic: user:pass"

# Custom header
KAELO_AUTH_API_EXAMPLE_COM="Header: X-API-Key: xxx"
```

### CLI commands

```bash
kaelo serve              # Start MCP server (stdio transport)
kaelo fetch <url>        # Fetch a URL and print extracted Markdown
kaelo prove-it           # Run self-test to verify installation
kaelo prove-it --challenge  # Test against a Cloudflare-protected URL
kaelo cache status       # Show cache size, entries, top domains
kaelo cache clear        # Clear everything
kaelo cache clear --domain X     # Clear specific domain
kaelo cache clear --older-than 24h  # Clear old entries
kaelo cache export <path>       # Export route cache strategies to JSON
kaelo cache import <path>       # Import route cache strategies from JSON
kaelo cache import-pack <path>  # Import community route pack
kaelo cache show-auth           # Show stored per-domain auth (tokens redacted)
kaelo cache forget-auth <domain> # Remove stored auth for a domain
```

## Architecture

Kaelo is a single Rust crate with a modular structure:

- **`extraction/`** — content extraction pipeline (Markdown conversion, boilerplate removal, SPA detection, quality gate)
- **`fetch/`** — multi-strategy backends (HTTP, TLS impersonation, headless browser, public APIs)
- **`mcp/`** — MCP server with tool definitions and stdio transport
- **`router/`** — strategy selection, fallback chain, route probing
- **`search/`** — web search backends (DuckDuckGo, SearXNG)
- **`storage/`** — SQLite storage (route cache, content cache, migrations)

### Documentation

- [Getting Started](./docs/getting-started.md) — install, configure, first run
- [Configuration](./docs/configuration.md) — all config options and env vars
- [Architecture](./docs/architecture.md) — technical deep-dive
- [MCP Tools](./docs/mcp-tools.md) — complete tool reference

The route cache sits at the center. On the first request to a domain, Kaelo probes strategies (fastest first) and caches the winner. On every subsequent request, it goes straight to the known-good strategy, cutting latency from ~8s to ~1.5s.

252 tests.

## Competitive Landscape

| | Kaelo | Fetcher MCP | markdown-for-agents | server-fetch |
|---|---|---|---|---|
| **Language** | Rust | TypeScript | TypeScript | TypeScript |
| **HTTP fetch** | ✅ | via Playwright | via Playwright | ✅ |
| **JS rendering** | ✅ chromiumoxide | ✅ Playwright | ✅ Playwright | ❌ |
| **TLS impersonation** | ✅ | ❌ | ❌ | ❌ |
| **Route cache** | ✅ SQLite | ❌ | ✅ LRU | ❌ |
| **Web search** | ✅ DuckDuckGo | ❌ | ✅ DuckDuckGo | ❌ |
| **Token-aware** | ✅ budget, focus | ❌ | ✅ content scoring | ❌ |
| **Local & free** | ✅ MIT | ✅ MIT | ✅ MIT | ✅ MIT |

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](./CONTRIBUTING.md) for guidelines.

## License

[MIT](./LICENSE)
