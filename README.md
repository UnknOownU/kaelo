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
| Existing tools are AGPL or paid | MIT licensed, 100% local, zero API keys |

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
# Install (when available)
cargo install kaelo

# Add to OpenCode
# In opencode.json:
{
  "mcp": {
    "kaelo": {
      "type": "local",
      "command": ["kaelo"],
      "enabled": true
    }
  }
}

# That's it. Kaelo auto-starts with your agent session.
```

## MCP Tools

| Tool | Description |
|---|---|
| `web_fetch(url, options)` | Fetch a URL, return clean Markdown |
| `web_search(query)` | Search the web, return results |
| `web_extract(url, query)` | Fetch and extract targeted content |

## Cache Management

```bash
kaelo cache status              # Show cache size, entries, top domains
kaelo cache clear               # Clear everything
kaelo cache clear --domain X    # Clear specific domain
kaelo cache clear --older-than 24h  # Clear old entries
```

## Configuration

Via environment variables or config file:

```
KAelo_CACHE_ENABLED=true        # Master cache switch
KAelo_CACHE_MAX_SIZE=52428800   # 50 MB hard limit
KAelo_CACHE_TTL=3600            # 1 hour default TTL
KAelo_CACHE_COMPRESSION=gzip    # Compress cached content
```

## Competitive Landscape

| | Kaelo | webclaw | insane-search | Firecrawl |
|---|---|---|---|---|
| **Language** | Rust | Rust | Python | TypeScript |
| **Intelligent routing** | ✅ | ❌ | ❌ | ❌ |
| **Learning cache** | ✅ | ❌ | ❌ | ❌ |
| **Token-aware** | ✅ | ❌ | ❌ | ❌ |
| **100% local** | ✅ | Mostly | ✅ | ❌ |
| **MCP server** | ✅ | ✅ | ❌ | ❌ |
| **License** | MIT | AGPL-3.0 | MIT | Commercial |
| **Price** | Free | Free | Free | $19-$499/mo |

## Status

**Pre-alpha.** In design phase. See [PRD.md](./PRD.md) for full product requirements.

## License

MIT
