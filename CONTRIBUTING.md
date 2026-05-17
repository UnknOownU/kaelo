# Contributing to Kaelo

## Development Setup

```bash
git clone https://github.com/hachemi/kaelo.git
cd kaelo
cargo build
cargo test --all
```

## Project Structure

```
src/
  main.rs              CLI entrypoint (clap)
  lib.rs               Module declarations and re-exports
  config.rs            Configuration (env vars, TOML, defaults)
  types.rs             Shared types (FetchRequest, FetchResponse, Strategy, FetchError)
  error.rs             Error types
  update.rs            Self-update from GitHub releases
  extraction/          Content extraction pipeline (Markdown, boilerplate removal, SPA detection)
  fetch/               FetchBackend trait + backends (HTTP, TLS impersonation, headless, public API)
  mcp/                 MCP server (rmcp, stdio transport, tool definitions)
  router/              Strategy selection and fallback chain
  search/              Web search backends (DuckDuckGo, SearXNG)
  storage/             SQLite storage (route cache, content cache, migrations)
```

## Development Commands

```bash
cargo build                                           # Build
cargo test                                            # Run all tests
cargo clippy --all-targets --all-features -- -D warnings  # Lint
cargo fmt --check                                     # Check formatting
cargo fmt                                             # Auto-format
cargo bench                                           # Run benchmarks
```

## Pull Request Process

1. Fork the repository
2. Create a feature branch: `git checkout -b feat/my-feature`
3. Make your changes
4. Ensure `cargo test`, `cargo clippy`, and `cargo fmt --check` all pass
5. Commit with conventional commits: `feat(scope): description`
6. Push and open a Pull Request

## Commit Style

Use conventional commits:
- `feat(scope): add new feature`
- `fix(scope): fix bug`
- `docs: update documentation`
- `chore: maintenance tasks`

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
