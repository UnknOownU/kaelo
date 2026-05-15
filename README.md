# Kaelo

> Intelligent web fetching for AI agents. Local-first, token-aware, learns as it goes.

## What is Kaelo?

Kaelo is a Rust MCP server that gives AI agents (OpenCode, Claude Code, Cursor, etc.) reliable access to web content — even when sites try to block them.

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
│   Router     │────▶│  Route Cache (SQLite)            │
│              │     │  reddit.com → TLS mobile (1.5s)  │
│              │     │  github.com → HTTP simple (0.3s) │
│              │     │  bloomberg.com → headless (4.8s) │
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

```bash
# Build from source
git clone https://github.com/HachemiH/kaelo.git
cd kaelo
cargo install --path .

# Or install via Homebrew
brew install --build-from-source packaging/homebrew/kaelo.rb

# Optional: for JS-heavy SPA sites (RSI Galactapedia, Bloomberg, etc.)
brew install --cask chromium
xattr -cr /Applications/Chromium.app  # macOS Gatekeeper fix
```

### CLI Commands

```bash
kaelo serve            # Start MCP server (for agent integration)
kaelo fetch <url>      # Fetch a URL and print extracted Markdown
kaelo prove-it         # Run self-test to verify installation
kaelo cache status     # Show cache size, entries, top domains
kaelo cache clear      # Clear everything
kaelo cache clear --domain X    # Clear specific domain
kaelo cache clear --older-than 24h  # Clear old entries
```

### MCP Configuration

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

**Windsurf** (`.windsurf/mcp.json`):

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

That's it. Kaelo auto-starts with your agent session.

## MCP Tools

**Available (v0.1):**

| Tool | Description | Key Parameters |
|---|---|---|
| `web_fetch` | Fetch a URL, return clean Markdown | url, strategy, token_budget, focus, no_cache |
| `web_search` | Search DuckDuckGo, optionally fetch top results | query, max_results, fetch_content |
| `cache_status` | Show cache statistics | — |
| `cache_clear` | Clear cached content | — |
| `ping` | Health check — returns pong | — |

## Backends

| Strategy | Backend | Use Case |
|---|---|---|
| `HttpSimple` | reqwest | Static sites, APIs |
| `TlsChrome` | wreq (TLS impersonation) | Cloudflare-protected sites |
| `TlsMobile` | wreq (mobile TLS) | Sites that block desktop bots |
| `Headless` | chromiumoxide | JS-heavy SPAs (React, Next.js, Vue) |
| `PublicApi` | (planned) | Reddit, HN, YouTube native APIs |

Kaelo auto-selects the best strategy per domain using its route cache. You can force a specific strategy via the `strategy` parameter:

```json
{"url": "https://example.com", "strategy": "Headless"}
```

**Headless browser** requires Chromium installed locally:
```bash
brew install --cask chromium
xattr -cr /Applications/Chromium.app
```

## Cache Management

```bash
kaelo cache status              # Show cache size, entries, top domains
kaelo cache clear               # Clear everything
kaelo cache clear --domain X    # Clear specific domain
kaelo cache clear --older-than 24h  # Clear old entries
```

## Configuration

Via environment variables or TOML config file (`~/.config/kaelo/config.toml`):

```toml
# ~/.config/kaelo/config.toml
cache_enabled = true
cache_max_size = 52428800   # 50 MB
cache_max_entry = 102400    # 100 KB per entry
cache_ttl = 3600            # 1 hour
cache_compression = "gzip"
db_path = "~/.config/kaelo/kaelo.db"
log_level = "info"
```

Environment variables (override config file):

```
KAELO_CACHE_ENABLED=true        # Master cache switch
KAELO_CACHE_MAX_SIZE=52428800   # 50 MB hard limit
KAELO_CACHE_TTL=3600            # 1 hour default TTL
KAELO_CACHE_COMPRESSION=gzip    # Compress cached content
```

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

## Status

**Beta (v0.1).** Core fetching (HTTP, TLS, headless browser), route cache, content cache, web search, MCP server, CLI — all functional. Tested against static sites, Cloudflare-protected sites, and JS-heavy SPAs (RSI Galactapedia).

See [PRD.md](./PRD.md) for full product vision.

## License

MIT
