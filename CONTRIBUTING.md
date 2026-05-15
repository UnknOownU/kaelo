# Contributing to Kaelo

## Development Setup

```bash
git clone https://github.com/hachemi/kaelo.git
cd kaelo
cargo build
cargo test --all
```

## Project Structure

| Crate | Description |
|---|---|
| `kaelo-core` | Storage (SQLite), Config, Router, Extraction pipeline |
| `kaelo-fetch` | FetchBackend trait + 3 backends (HTTP, TLS impersonation, Headless) |
| `kaelo-mcp` | MCP server using rmcp (stdio transport) |
| `kaelo-cli` | CLI binary using clap |

## Development Commands

```bash
cargo build                    # Build all crates
cargo test --all               # Run all tests
cargo test -p kaelo-core       # Run kaelo-core tests only
cargo clippy --all-targets --all-features -- -D warnings  # Lint
cargo fmt --check              # Check formatting
cargo fmt                      # Auto-format
cargo bench -p kaelo-core      # Run benchmarks
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
