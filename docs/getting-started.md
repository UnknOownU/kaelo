# Getting Started

A step-by-step guide to installing, configuring, and running Kaelo for the first time.

## Prerequisites

Kaelo runs on macOS and Linux. You'll need one of the following combinations:

| Platform | Architecture | Notes |
|---|---|---|
| macOS | ARM64 (Apple Silicon) | Tested on macOS 14+ |
| macOS | x86_64 (Intel) | Tested on macOS 13+ |
| Linux | x86_64 | Requires OpenSSL dev headers for build |
| Linux | ARM64 | Requires OpenSSL dev headers for build |

**Rust 1.76+** is required only if building from source. The one-liner and Homebrew installs bundle a prebuilt binary.

**Chromium** is optional but recommended if you need to fetch JS-heavy single-page apps (React, Next.js, Vue, Blazor). See [Chromium for SPAs](#optional-chromium-for-spas) below.

## Installation

Pick the method that fits your workflow. The one-liner and Homebrew installs place the binary at `~/.local/bin/kaelo`; building from source installs to `~/.cargo/bin/kaelo`.

### Option 1: One-liner (Recommended)

The fastest path. Downloads the prebuilt binary for your platform and places it in `~/.local/bin/`.

```bash
curl -fsSL https://raw.githubusercontent.com/HachemiH/kaelo/main/install.sh | bash
```

Make sure `~/.local/bin` is on your `PATH`. Add this to your shell profile if it isn't already:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

### Option 2: Build from source

Clone the repo and compile locally. This takes a few minutes but gives you the latest development version.

```bash
git clone https://github.com/HachemiH/kaelo.git
cd kaelo
cargo install --path .
```

The binary lands at `~/.cargo/bin/kaelo`. Rust's toolchain directory is usually on `PATH` already if you have Rust installed.

### Option 3: Homebrew

Build from the Homebrew formula. Useful if you manage all your tools through Homebrew.

```bash
brew install --build-from-source packaging/homebrew/kaelo.rb
```

### Optional: Chromium for SPAs

Kaelo can use a headless Chromium instance to render JavaScript-heavy sites that return blank pages with plain HTTP requests. This covers sites like Bloomberg, RSI Galactapedia, and most modern SPAs.

On macOS:

```bash
brew install --cask chromium
xattr -cr /Applications/Chromium.app
```

The `xattr` command clears macOS Gatekeeper quarantine flags. Without it, headless mode fails silently because macOS blocks unsigned apps from launching programmatically.

On Linux, install Chromium through your package manager (e.g., `apt install chromium-browser` or `pacman -S chromium`). Make sure the `chromium` or `chromium-browser` binary is on `PATH`.

## Verify Installation

Run the built-in self-test to confirm everything works:

```bash
kaelo prove-it
```

This fetches a public URL using each strategy and reports results. A passing test means the binary, network access, and content extraction pipeline are all working.

To also verify that TLS impersonation can handle Cloudflare-protected sites, run the extended challenge:

```bash
kaelo prove-it --challenge
```

This hits a Cloudflare-protected URL. If it fails, TLS impersonation isn't working on your system, which means Cloudflare-backed sites will fall back to the headless browser (if Chromium is installed) or plain HTTP (which may return a block page).

## MCP Client Setup

Kaelo runs as an MCP server over stdio. It starts automatically when your agent launches, so you just need to add it to your config once.

### OpenCode

Add to `opencode.json` at your project root:

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

No explicit `serve` argument needed. OpenCode invokes the command as an MCP server directly.

### Claude Code

Add to `.claude/settings.json`:

```jsonc
{
  "mcpServers": {
    "kaelo": {
      "command": "kaelo"
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json`:

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

Cursor requires the explicit `serve` subcommand.

After adding the config, restart your agent or start a new session. You can confirm Kaelo is connected by checking the agent's MCP server list or running the ping tool (`kaelo ping` should return `pong`).

## First Steps

Once Kaelo is running as an MCP server in your agent, these tools are available:

### Fetch a URL

Use the `web_fetch` tool from your agent, or test directly from the CLI:

```bash
kaelo fetch https://example.com
```

This returns clean Markdown extracted from the page. On the first visit to a domain, Kaelo probes different strategies (HTTP, TLS impersonation, headless browser) to find the fastest one that works. That result gets cached, so subsequent visits to the same domain skip the probing step entirely.

You can control output size with a token budget:

```bash
kaelo fetch https://example.com --token-budget 500
```

Or focus on a specific section using a CSS selector:

```bash
kaelo fetch https://example.com --focus "article.main"
```

### Search the web

Use the `web_search` tool from your agent, or from the CLI:

```bash
kaelo search "rust async patterns"
```

By default, Kaelo uses DuckDuckGo. No API key needed. To use a self-hosted SearXNG instance instead, set the `KAELO_SEARXNG_URL` environment variable or configure it in `~/.config/kaelo/config.toml`. See the [Configuration Guide](./configuration.md) for details.

Search results return titles, URLs, and snippets. Set `fetch_content: true` in the MCP tool call to automatically fetch the top result's full content in a single round trip.

### Check the cache

The route cache is where Kaelo remembers which fetch strategy works for each domain. Check its status:

```bash
kaelo cache status
```

This shows cache size, entry count, and the top domains by hit frequency. The cache lives at `~/.config/kaelo/kaelo.db` (SQLite).

Clear the cache if strategies have changed or you want to re-probe everything:

```bash
kaelo cache clear
```

You can also clear entries for a specific domain:

```bash
kaelo cache clear --domain reddit.com
```

## Next Steps

Now that Kaelo is installed and working, here's where to go next:

- **[Configuration Guide](./configuration.md)** — Customize cache behavior, search backends, per-domain authentication, and environment variable overrides.
- **[Architecture](./architecture.md)** — Understand how the router, route cache, fetch backends, and token-aware extraction layer work together.
- **[MCP Tools Reference](./mcp-tools.md)** — Complete parameter docs for `web_fetch`, `web_search`, `fetch_urls`, and `ping`, including examples for each tool.
