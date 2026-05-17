# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.0.x   | :white_check_mark: |
| < 1.0   | :x:                |

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please:

1. Open a GitHub issue using the **Security** tab on the repository, or
2. Email the maintainer directly if the issue is sensitive

You should receive a response within 48 hours. If the issue is confirmed,
a patch will be released as soon as possible depending on complexity.

## Security Features

Kaelo is designed with security in mind:

- **Local-first**: All data stays on your machine (SQLite, config files)
- **No telemetry**: Kaelo never phones home or sends data to external services
- **Token redaction**: Auth tokens are redacted in logs and `cache show-auth` output
- **No cloud dependencies**: Fetch operations use direct HTTP/TLS connections
- **Configurable auth**: Per-domain authentication with environment variable support
