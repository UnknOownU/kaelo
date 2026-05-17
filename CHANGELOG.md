# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-05-17

### Changed
- **Breaking**: Merged the 4-crate workspace into a single `kaelo` crate for simpler installation and publishing
- `cargo install kaelo` now works directly (previously required cloning the repo)
- Removed workspace structure (`kaelo-core`, `kaelo-fetch`, `kaelo-mcp`, `kaelo-cli` → single crate with modules)

### Note
This is the first stable release. The v0.x line established the core features (multi-strategy fetching, SPA detection, route cache, MCP server). v1.0.0 consolidates everything into a clean, publishable package.

## [0.4.2] - 2026-05-17

### Added
- Response metadata in MCP output: strategy name, timing, cache status prepended as `⚙ Kaelo:` line in content
- `_meta` field in MCP tool responses with structured strategy and timing information

### Fixed
- Corrected quality gate if/else structure for cached content validation
- `cargo fmt` pass

## [0.4.1] - 2026-05-17

### Added
- Startup update notice: appends a non-blocking message to MCP responses when a newer version is available on GitHub

### Fixed
- Cached content quality validation: stale or empty cached entries are now invalidated and re-fetched
- Headless wait time increased for Blazor and other heavy SPA frameworks
- Log file opened in append mode (`File::create` → `OpenOptions::append`) to preserve logs across restarts

## [0.4.0] - 2026-05-17

### Added
- SPA-aware fetch pipeline: detects JavaScript-heavy sites and fast-paths to the Headless backend
- SPA fingerprint detection module: recognizes Blazor Server, Next.js, Vue, Angular, React, Svelte, Nuxt, Remix, Astro, and generic SPA indicators
- Quality gate overhaul with SPA-aware heuristics: minimum 200 chars, 5 non-empty lines, HTML artifact rejection

### Changed
- Fallback chain now routes SPA-detected domains directly to Headless, skipping HTTP/TLS probing
- Content quality validation applies to both fresh and cached responses

## [0.3.0] - 2026-05-16

### Added
- `kaelo update` command: check for newer versions and self-update from GitHub releases
- `kaelo update --check`: check only, don't install
- `kaelo update --force`: reinstall even if already up-to-date
- Startup version check in `kaelo serve`: non-blocking notification when a new version is available (24h cache)
- Version info in MCP `ping` response (JSON with status and version fields)
- Install method detection: refuses self-update for Homebrew/Cargo installs with helpful message
- `kaelo-core::update` module: GitHub releases API integration, SHA256 checksum verification, binary extraction

### Fixed
- Content cache now keyed by (URL, strategy) — each fetch strategy gets its own cache entry, preventing cross-strategy pollution
- Fallback chain is now active in all MCP handlers (web_fetch, fetch_urls, web_search) — retries with next strategy on 403/timeout
- Content quality validation gate: content < 50 chars or < 3 non-empty lines is rejected and triggers fallback
- PublicApi added to fallback chain (5th position, last resort)
- Terminal errors (404, 429) return immediately without retry

### Changed
- `reqwest` promoted to workspace-shared dependency
- Added `semver` crate for proper version comparison

### Technical
- 272 tests (9 new for update module, 5 new integration tests for cache routing)
- SQLite migration V003: composite primary key (url, strategy) on url_cache table
- AI slop cleanup across codebase

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
- Rust workspace with 4 crates (kaelo-core, kaelo-fetch, kaelo-mcp, kaelo)
- 252 tests
- CI/CD via GitHub Actions
- Homebrew formula and install.sh installer
