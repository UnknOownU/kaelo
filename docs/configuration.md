# Configuration

Kaelo reads configuration from a TOML file and environment variables. When both are set for the same option, the environment variable wins.

## Config File Location

```
~/.config/kaelo/config.toml
```

The file is not created automatically. If it doesn't exist, Kaelo falls back to environment variables and built-in defaults.

## All Options

### Cache Settings

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `cache_enabled` | bool | `true` | Whether content caching is active. Set to `false` to always fetch fresh content. |
| `cache_max_size` | integer | `52428800` | Maximum total cache size in bytes (50 MB). |
| `cache_max_entry` | integer | `102400` | Maximum size of a single cached entry in bytes (100 KB). Must be less than or equal to `cache_max_size`. |
| `cache_ttl` | integer | `3600` | Time-to-live for cached entries, in seconds (1 hour). |
| `cache_compression` | string | `"gzip"` | Compression algorithm for cached content. Accepted values: `gzip`, `none`. |

### Database

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `db_path` | string | `~/.config/kaelo/kaelo.db` | Path to the SQLite database file. |

### Logging

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `log_level` | string | `"info"` | Log verbosity. Accepted values: `trace`, `debug`, `info`, `warn`, `error`. |

### Search

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `search_backend` | string | *(none, uses DuckDuckGo)* | Search backend to use. Set to `"searxng"` to use a self-hosted SearXNG instance. When omitted, DuckDuckGo is used. |
| `searxng_url` | string | *(none)* | URL of your SearXNG instance. Required when `search_backend` is `"searxng"`. |

### Strategy

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `default_strategy` | string | *(none, auto-detects)* | Override automatic strategy detection for all requests. See [Strategy Override](#strategy-override) for details. |

## Environment Variables

Every config file option has a corresponding `KAELO_` environment variable. These always take priority over the config file.

| Environment Variable | Config Option | Type | Default |
|---------------------|---------------|------|---------|
| `KAELO_CACHE_ENABLED` | `cache_enabled` | bool (`true`/`1`) | `true` |
| `KAELO_CACHE_MAX_SIZE` | `cache_max_size` | integer (bytes) | `52428800` |
| `KAELO_CACHE_MAX_ENTRY` | `cache_max_entry` | integer (bytes) | `102400` |
| `KAELO_CACHE_TTL` | `cache_ttl` | integer (seconds) | `3600` |
| `KAELO_CACHE_COMPRESSION` | `cache_compression` | string | `gzip` |
| `KAELO_DB_PATH` | `db_path` | path | `~/.config/kaelo/kaelo.db` |
| `KAELO_LOG_LEVEL` | `log_level` | string | `info` |
| `KAELO_SEARCH_BACKEND` | `search_backend` | string | *(DuckDuckGo)* |
| `KAELO_SEARXNG_URL` | `searxng_url` | string (URL) | *(none)* |
| `KAELO_DEFAULT_STRATEGY` | `default_strategy` | string | *(none)* |

Boolean env vars accept `true` or `1` as truthy values. Everything else is treated as false.

## Per-Domain Authentication

Kaelo supports attaching authentication credentials to specific domains through environment variables. This is useful for fetching content from private APIs, internal tools, or sites behind basic auth.

The pattern is `KAELO_AUTH_<DOMAIN>` where the domain uses underscores in place of dots. For example, `api.github.com` becomes `KAELO_AUTH_API_GITHUB_COM`.

Three auth methods are supported:

### Bearer Token

```bash
KAELO_AUTH_API_GITHUB_COM="Bearer: ghp_xxxxxxxxxxxx"
```

### Basic Auth

```bash
KAELO_AUTH_PRIVATE_SITE_COM="Basic: username:password"
```

### Custom Header

```bash
KAELO_AUTH_API_INTERNAL_IO="Header: X-API-Key:your-key-here"
```

You can set multiple auth entries for different domains at the same time. Only one credential type per domain is supported in the current format (last one wins if you set multiple `KAELO_AUTH_` vars for the same domain with different schemes).

## Search Backends

### DuckDuckGo (default)

No configuration needed. Kaelo scrapes DuckDuckGo's HTML results directly. This works out of the box but may be rate-limited or blocked in some environments.

### SearXNG

SearXNG is a privacy-respecting meta search engine you can self-host. To use it:

1. Run a SearXNG instance (via Docker, for example):

```bash
docker run -d -p 8888:8080 searxng/searxng
```

2. Point Kaelo to it:

```toml
search_backend = "searxng"
searxng_url = "http://localhost:8888"
```

Or via environment variables:

```bash
export KAELO_SEARCH_BACKEND=searxng
export KAELO_SEARXNG_URL=http://localhost:8888
```

## Strategy Override

By default, Kaelo auto-detects the best fetch strategy for each URL by probing the target server. The available strategies are:

| Strategy | Description |
|----------|-------------|
| `HttpSimple` | Plain HTTP client. Fastest option, works for most public sites. |
| `TlsChrome` | HTTP client with a Chrome TLS fingerprint. Bypasses basic bot detection. |
| `TlsMobile` | HTTP client with a mobile TLS fingerprint. |
| `Headless` | Full headless browser. Slowest but handles JavaScript-rendered pages. |
| `PublicApi` | Treats the URL as a public API endpoint. |

Set `default_strategy` to skip auto-detection and force a specific strategy for every request:

```toml
# Force headless browser for everything (slow but thorough)
default_strategy = "Headless"
```

```bash
# Force simple HTTP for everything (fast but may fail on protected sites)
KAELO_DEFAULT_STRATEGY=HttpSimple
```

This is useful when you know all your target sites behave the same way, or when you want to avoid the overhead of the probing step.

## Example Configurations

### Minimal (defaults are fine)

```toml
# Nothing to configure. Kaelo works out of the box
# with DuckDuckGo search, gzip caching, and auto-detected strategies.
```

### SearXNG with longer cache

```toml
search_backend = "searxng"
searxng_url = "http://localhost:8888"

cache_ttl = 86400       # Cache for 24 hours
cache_max_size = 104857600  # 100 MB cache
```

### Production with private API access

```toml
log_level = "warn"
cache_enabled = true
cache_max_size = 209715200   # 200 MB
cache_ttl = 7200             # 2 hours
default_strategy = "TlsChrome"
db_path = "/var/lib/kaelo/kaelo.db"
```

```bash
# In your environment or systemd unit file
KAELO_AUTH_API_GITHUB_COM="Bearer: ghp_xxxxxxxxxxxx"
KAELO_AUTH_STAGING_MYAPP_COM="Basic: deploy:secret-password"
```

### Debugging

```toml
log_level = "trace"
cache_enabled = false
```

Disabling the cache and setting the most verbose log level makes it easy to see exactly what Kaelo is doing on each request.
