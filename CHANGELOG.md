# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2025-05-16

### Added
- Multi-strategy fetch with route cache (HttpSimple, TlsChrome, TlsMobile, Headless, PublicApi)
- SQLite-backed route cache that learns which strategy works per domain
- Token-aware content extraction with budget limits, deduplication, and CSS selector focusing
- Web search via DuckDuckGo or self-hosted SearXNG
- Batch URL fetching (up to 10 URLs in parallel)
- MCP server over stdio (MCP protocol 2024-11-05)
- Per-domain authentication (Bearer, Basic, custom headers)
- Browser session support for cookie-aware navigation
- Content cache with gzip compression, TTL, and size limits
- Cache management CLI: status, clear, export, import, import-pack
- `kaelo prove-it` self-test command
- `kaelo setup` interactive configuration
- CLI interface with clap

### Technical
- Rust workspace with 4 crates (kaelo-core, kaelo-fetch, kaelo-mcp, kaelo-cli)
- 252 tests
- CI/CD via GitHub Actions
- Homebrew formula and install.sh installer
