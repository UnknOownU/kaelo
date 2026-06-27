use clap::{Parser, Subcommand};
use kaelo::fetch::backend::FetchBackend;
use kaelo::update;
use std::path::PathBuf;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "kaelo",
    version,
    about = "Intelligent web fetcher for AI agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start MCP server (stdio transport by default)
    Serve {
        /// Enable HTTP transport instead of stdio
        #[arg(long)]
        http: bool,
        /// Port for HTTP server (default: 8080)
        #[arg(long, default_value = "8080")]
        port: u16,
        /// Host bind address for HTTP server (default: 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
    },
    /// Cache management
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
    /// Fetch a URL and return Markdown
    Fetch {
        /// URL to fetch
        url: String,
    },
    /// Print setup instructions and current configuration
    Setup,
    /// Run a demo to verify installation works
    ProveIt {
        /// Run a harder challenge (Cloudflare-protected URL)
        #[arg(long)]
        challenge: bool,
    },
    /// Check for updates and optionally self-update
    Update {
        /// Only check for updates, don't install
        #[arg(long)]
        check: bool,
        /// Reinstall even if already up-to-date
        #[arg(long)]
        force: bool,
    },
    /// Run health checks on the Kaelo installation
    Doctor,
}

#[derive(Subcommand)]
enum CacheCommands {
    /// Show cache status
    Status,
    /// Clear cache
    Clear,
    /// Export route cache strategies to a JSON file
    Export {
        /// Output file path
        path: PathBuf,
    },
    /// Import route cache strategies from a JSON file
    Import {
        /// Input file path
        path: PathBuf,
    },
    /// Import a community route pack from a JSON file
    ImportPack {
        /// Input pack file path
        path: PathBuf,
    },
    /// List domains with auth configured (tokens redacted)
    ShowAuth,
    /// Remove auth config for a domain
    ForgetAuth {
        /// Domain to remove auth for
        domain: String,
    },
}

fn init_tracing() {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let log_dir = PathBuf::from(home).join(".config").join("kaelo");
    let _ = std::fs::create_dir_all(&log_dir);

    let log_path = log_dir.join("kaelo.log");

    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "warning: cannot create log file {}: {e}",
                log_path.display()
            );
            tracing_subscriber::fmt()
                .with_env_filter(
                    EnvFilter::try_from_env("KAELO_LOG_LEVEL")
                        .unwrap_or_else(|_| EnvFilter::new("info")),
                )
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .init();
            return;
        }
    };

    let filter =
        EnvFilter::try_from_env("KAELO_LOG_LEVEL").unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(std::sync::Mutex::new(file));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .pretty()
        .with_writer(std::io::stderr);

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .init();
}

/// Detect Chrome/Chromium across platforms, mirroring chromiumoxide's own
/// detection order: `$CHROME` env var first, then well-known install paths,
/// then `which <name>` on Linux. Returns the resolved path if found.
///
/// On Windows the previous implementation shelled out to `which`, which does
/// not exist there, so `doctor` always reported Chrome as missing.
fn detect_chrome() -> Option<std::path::PathBuf> {
    // 1. `$CHROME` env var — same priority as chromiumoxide's detection.
    if let Ok(path) = std::env::var("CHROME") {
        let p = PathBuf::from(path);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Well-known install locations, checked in priority order.
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(pf) = std::env::var("ProgramFiles") {
            candidates.push(PathBuf::from(&pf).join("Google/Chrome/Application/chrome.exe"));
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            candidates.push(PathBuf::from(&pf86).join("Google/Chrome/Application/chrome.exe"));
        }
        if let Ok(la) = std::env::var("LOCALAPPDATA") {
            candidates.push(PathBuf::from(&la).join("Google/Chrome/Application/chrome.exe"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ));
        candidates.push(PathBuf::from(
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for name in &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ] {
            if let Ok(out) = std::process::Command::new("which")
                .arg(name)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
            {
                if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout);
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        candidates.push(PathBuf::from(trimmed));
                    }
                }
            }
        }
    }

    candidates.into_iter().find(|p| p.exists())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    tracing::info!("Kaelo v{} starting...", env!("CARGO_PKG_VERSION"));
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { http, port, host } => {
            let config = kaelo::config::Config::load();
            let storage = kaelo::storage::Storage::open(&config.db_path.to_string_lossy())?;
            let server = kaelo::mcp::KaeloServer::with_config(
                storage,
                config.searxng_url.clone(),
                config.search_backend.clone(),
            );

            {
                let cache_path = config
                    .db_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new("."))
                    .join(".update-check");

                if kaelo::update::should_check(&cache_path) {
                    tokio::spawn(async move {
                        match kaelo::update::check_for_update().await {
                            Ok(Some(info)) => {
                                tracing::warn!(
                                    "Update available: v{} → v{} — {}",
                                    info.current,
                                    info.latest,
                                    info.release_url
                                );
                            }
                            Ok(None) => {}
                            Err(e) => {
                                tracing::debug!("Update check failed: {e}");
                            }
                        }
                    });
                    if let Err(e) = kaelo::update::write_check_cache(&cache_path) {
                        tracing::debug!("Failed to write update cache: {e}");
                    }
                }
            }

            if http {
                serve_http(server, &host, port).await?;
            } else {
                tokio::select! {
                    result = kaelo::mcp::server::serve_stdio(server) => {
                        result?;
                    }
                    _ = tokio::signal::ctrl_c() => {
                        tracing::info!("Received Ctrl+C, shutting down gracefully...");
                        let cleanup = async {
                            kaelo::fetch::backends::shutdown_browser_pool().await;
                        };
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(5),
                            cleanup,
                        ).await;
                        tracing::info!("Shutdown complete.");
                    }
                }
            }
        }
        Commands::Cache { command } => {
            let config = kaelo::config::Config::load();
            match command {
                CacheCommands::Status => {
                    match kaelo::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo::storage::route_cache::RouteCache::new(&storage);
                            let content_cache =
                                kaelo::storage::content_cache::ContentCache::new(&storage);

                            let route_count = route_cache.count_entries().unwrap_or(0);
                            let domains = route_cache.get_all_domains().unwrap_or_default();
                            let stats = content_cache.stats().unwrap_or_else(|_| {
                                kaelo::storage::content_cache::CacheStats {
                                    entry_count: 0,
                                    total_size_bytes: 0,
                                    top_domains: vec![],
                                }
                            });

                            println!("Route Cache:");
                            println!("  Entries: {}", route_count);
                            println!("  Domains: {:?}", domains);
                            println!("Content Cache:");
                            println!("  Entries: {}", stats.entry_count);
                            println!("  Size: {} bytes", stats.total_size_bytes);
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::Clear => {
                    match kaelo::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let content_cache =
                                kaelo::storage::content_cache::ContentCache::new(&storage);
                            content_cache.clear_all()?;
                            println!("Cache cleared.");
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::Export { path } => {
                    match kaelo::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo::storage::route_cache::RouteCache::new(&storage);
                            route_cache.export_json(&path)?;
                            println!("Exported route cache to {}", path.display());
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::Import { path } => {
                    match kaelo::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo::storage::route_cache::RouteCache::new(&storage);
                            let count = route_cache.import_json(&path)?;
                            println!("Imported {} strategies from {}", count, path.display());
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::ImportPack { path } => {
                    match kaelo::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo::storage::route_cache::RouteCache::new(&storage);
                            let count = route_cache.import_pack(&path)?;
                            println!("Imported {} strategies from pack {}", count, path.display());
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::ShowAuth => {
                    println!("Auth config is read from KAELO_AUTH_* env vars.");
                    println!("No auth domains currently configured.");
                }
                CacheCommands::ForgetAuth { domain } => {
                    println!("Auth config is read from KAELO_AUTH_* env vars.");
                    println!("Unset the relevant variable to remove auth for {}", domain);
                }
            }
        }
        Commands::Fetch { url } => {
            let backend = kaelo::fetch::backends::HttpSimple::new()
                .map_err(|e| anyhow::anyhow!("Failed to create backend: {}", e))?;

            let request = kaelo::types::FetchRequest {
                url: url.clone(),
                headers: std::collections::HashMap::new(),
                timeout: std::time::Duration::from_secs(30),
                follow_redirects: true,
            };

            let response = backend
                .fetch(request)
                .await
                .map_err(|e| anyhow::anyhow!("Fetch failed: {:?}", e))?;

            let html = String::from_utf8_lossy(&response.body);
            let extracted = kaelo::extraction::extract(&html, &url, &response.content_type)?;

            match extracted {
                Some(content) => print!("{}", content.text_content),
                None => print!("{}", html),
            }
        }
        Commands::Setup => {
            let config = kaelo::config::Config::load();
            println!("Kaelo Configuration:");
            println!("  Cache enabled: {}", config.cache_enabled);
            println!("  Cache max size: {} bytes", config.cache_max_size);
            println!("  Cache TTL: {} seconds", config.cache_ttl);
            println!("  Database: {:?}", config.db_path);
            println!("  Log level: {}", config.log_level);
            println!();

            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());

            struct Client {
                name: &'static str,
                config_path: std::path::PathBuf,
                config_json: &'static str,
            }

            let claude_config = if cfg!(target_os = "macos") {
                std::path::PathBuf::from(&home)
                    .join("Library/Application Support/Claude/claude_desktop_config.json")
            } else {
                std::path::PathBuf::from(&home).join(".config/Claude/claude_desktop_config.json")
            };

            let clients = [
                Client {
                    name: "Claude Desktop",
                    config_path: claude_config,
                    config_json: r#"{
  "mcpServers": {
    "kaelo": {
      "command": "kaelo",
      "args": ["serve"]
    }
  }
}"#,
                },
                Client {
                    name: "Cursor",
                    config_path: std::path::PathBuf::from(&home).join(".cursor/mcp.json"),
                    config_json: r#"{
  "mcpServers": {
    "kaelo": {
      "command": "kaelo",
      "args": ["serve"]
    }
  }
}"#,
                },
                Client {
                    name: "Windsurf",
                    config_path: std::path::PathBuf::from(&home).join(".windsurf/mcp.json"),
                    config_json: r#"{
  "mcpServers": {
    "kaelo": {
      "command": "kaelo",
      "args": ["serve"]
    }
  }
}"#,
                },
                Client {
                    name: "OpenCode",
                    config_path: std::path::PathBuf::from("opencode.json"),
                    config_json: r#"{
  "mcp": {
    "kaelo": {
      "type": "local",
      "command": ["kaelo"],
      "enabled": true
    }
  }
}"#,
                },
                Client {
                    name: "Zed",
                    config_path: std::path::PathBuf::from(&home).join(".config/zed/settings.json"),
                    config_json: r#"// Add to the "mcp" section of your settings.json:
{
  "mcp": {
    "servers": {
      "kaelo": {
        "command": "kaelo",
        "args": ["serve"]
      }
    }
  }
}"#,
                },
            ];

            println!("MCP Client Detection:");
            let mut detected_count = 0;
            for client in &clients {
                let exists = client.config_path.exists();
                let status = if exists { "\u{2713}" } else { " " };
                println!(
                    "  [{status}] {:20} {}",
                    client.name,
                    client.config_path.display()
                );
                if exists {
                    detected_count += 1;
                }
            }
            println!("  ({}/{} clients detected)", detected_count, clients.len());
            println!();

            println!("MCP Server Configs (copy the block for your client):\n");

            for client in &clients {
                let marker = if client.config_path.exists() {
                    "\u{2713}"
                } else {
                    " "
                };
                println!(
                    "[{marker}] {} — {}",
                    client.name,
                    client.config_path.display()
                );
                println!("{}", client.config_json);
                println!();
            }

            println!("Environment Variables:");
            println!("  KAELO_CACHE_ENABLED   - Enable/disable cache (default: true)");
            println!("  KAELO_CACHE_MAX_SIZE  - Max cache size in bytes (default: 52428800)");
            println!("  KAELO_CACHE_TTL       - Cache TTL in seconds (default: 3600)");
            println!("  KAELO_DB_PATH         - Database path (default: ~/.config/kaelo/kaelo.db)");
            println!("  KAELO_LOG_LEVEL       - Log level (default: info)");
        }
        Commands::ProveIt { challenge } => {
            println!("Kaelo prove-it: Verifying installation...\n");

            let backend = kaelo::fetch::backends::HttpSimple::new()
                .map_err(|e| anyhow::anyhow!("Backend failed: {}", e))?;
            println!("✓ HTTP backend initialized (HttpSimple)");

            let (demo_url, label) = if challenge {
                println!("Challenge mode: attempting a harder, Cloudflare-protected URL...");
                ("https://nowsecure.nl", "nowsecure.nl (Cloudflare)")
            } else {
                ("https://httpbin.org/get", "httpbin.org")
            };

            print!("→ Fetching {}... ", label);

            let request = kaelo::types::FetchRequest {
                url: demo_url.to_string(),
                headers: std::collections::HashMap::new(),
                timeout: std::time::Duration::from_secs(10),
                follow_redirects: true,
            };

            match backend.fetch(request).await {
                Ok(response) => {
                    println!(
                        "✓ (status {}, {} bytes)",
                        response.status,
                        response.body.len()
                    );
                    if challenge {
                        println!(
                            "  Note: HttpSimple may be blocked by Cloudflare. \
                            Enable the tls-impersonation feature for TlsChrome support."
                        );
                    }
                }
                Err(e) => {
                    println!("✗ {:?}", e);
                    return Err(anyhow::anyhow!("Fetch failed"));
                }
            }

            print!("→ Testing extraction pipeline... ");
            let html = r#"<html><body><article><h1>Test</h1><p>Hello from Kaelo!</p></article></body></html>"#;
            match kaelo::extraction::extract(html, "https://example.com", "text/html") {
                Ok(Some(content)) => {
                    println!("✓ (extracted {} chars)", content.text_content.len());
                }
                Ok(None) => println!("✗ No content extracted"),
                Err(e) => println!("✗ {}", e),
            }

            print!("→ Testing storage... ");
            match kaelo::storage::Storage::open(":memory:") {
                Ok(_) => println!("✓"),
                Err(e) => println!("✗ {}", e),
            }

            println!("\nAll checks passed! Kaelo is ready to use.");
            println!("Run `kaelo serve` to start the MCP server.");
        }
        Commands::Update { check, force } => {
            let method = update::detect_install_method();
            match method {
                update::InstallMethod::Homebrew => {
                    println!("Kaelo was installed via Homebrew.");
                    println!("Run: brew upgrade kaelo");
                    return Ok(());
                }
                update::InstallMethod::Cargo => {
                    println!("Kaelo was installed via cargo.");
                    println!("Run: cargo install kaelo");
                    return Ok(());
                }
                _ => {}
            }

            println!("Checking for updates...");

            match update::check_for_update().await {
                Ok(Some(update_info)) => {
                    println!(
                        "Update available: v{} → v{}",
                        update_info.current, update_info.latest
                    );
                    println!("Release: {}", update_info.release_url);

                    if let Some(notes) = &update_info.release_notes {
                        let preview: Vec<&str> = notes.lines().take(3).collect();
                        if !preview.is_empty() {
                            println!();
                            for line in preview {
                                println!("  {}", line);
                            }
                            if notes.lines().count() > 3 {
                                println!("  ... (see release page for full notes)");
                            }
                        }
                    }

                    if check {
                        return Ok(());
                    }

                    print!("\nDownload and install v{}? [y/N] ", update_info.latest);
                    use std::io::{self, BufRead, Write};
                    io::stdout().flush()?;
                    let mut input = String::new();
                    io::stdin().lock().read_line(&mut input)?;

                    if !input.trim().eq_ignore_ascii_case("y") {
                        println!("Update cancelled.");
                        return Ok(());
                    }

                    let target = update::platform_target()?;
                    println!("Downloading for {}...", target);

                    match update::download_update(&target, &format!("v{}", update_info.latest))
                        .await
                    {
                        Ok(binary_path) => {
                            let current_exe = std::env::current_exe()?;

                            match std::fs::copy(&binary_path, &current_exe) {
                                Ok(_) => {
                                    println!(
                                        "Updated to v{}. Restart Kaelo to use the new version.",
                                        update_info.latest
                                    );
                                }
                                Err(e) => {
                                    println!("Failed to replace binary: {e}");
                                    println!("The new binary is at: {}", binary_path.display());
                                    println!(
                                        "Install manually: cp {} {}",
                                        binary_path.display(),
                                        current_exe.display()
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Update failed: {e}");
                            std::process::exit(1);
                        }
                    }
                }
                Ok(None) => {
                    if force {
                        let current = update::current_version();
                        let target = update::platform_target()?;
                        println!("Already up to date (v{}). Force reinstalling...", current);
                        println!("Downloading for {}...", target);

                        match update::download_update(&target, &format!("v{}", current)).await {
                            Ok(binary_path) => {
                                let current_exe = std::env::current_exe()?;
                                match std::fs::copy(&binary_path, &current_exe) {
                                    Ok(_) => println!("Reinstalled v{}.", current),
                                    Err(e) => {
                                        println!("Failed to replace binary: {e}");
                                        println!("New binary at: {}", binary_path.display());
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!("Download failed: {e}");
                                std::process::exit(1);
                            }
                        }
                    } else {
                        println!("Already up to date (v{}).", update::current_version());
                    }
                }
                Err(e) => {
                    eprintln!("Unable to check for updates: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::Doctor => {
            let mut all_ok = true;

            match detect_chrome() {
                Some(path) => {
                    println!("\u{2713} Chromium/Chrome installed ({})", path.display());
                }
                None => {
                    println!("\u{2717} Chromium/Chrome not found");
                    all_ok = false;
                }
            }

            let config = kaelo::config::Config::load();
            match kaelo::storage::Storage::open(&config.db_path.to_string_lossy()) {
                Ok(storage) => {
                    let route_cache = kaelo::storage::route_cache::RouteCache::new(&storage);
                    match route_cache.count_entries() {
                        Ok(count) => {
                            println!(
                                "\u{2713} SQLite accessible ({}, {} route entries)",
                                config.db_path.display(),
                                count
                            );
                        }
                        Err(e) => {
                            println!("\u{2717} SQLite query failed: {e}");
                            all_ok = false;
                        }
                    }
                }
                Err(e) => {
                    println!("\u{2717} SQLite not accessible: {e}");
                    all_ok = false;
                }
            }

            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let config_path = std::path::PathBuf::from(&home)
                .join(".config")
                .join("kaelo")
                .join("kaelo.toml");
            match std::fs::read_to_string(&config_path) {
                Ok(_) => {
                    println!("\u{2713} Config readable ({})", config_path.display());
                }
                Err(_) => {
                    println!(
                        "\u{2713} Config not found ({}) — using defaults",
                        config_path.display()
                    );
                }
            }

            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                kaelo::fetch::backends::HttpSimple::new()?.fetch(kaelo::types::FetchRequest {
                    url: "https://example.com".to_string(),
                    headers: std::collections::HashMap::new(),
                    timeout: std::time::Duration::from_secs(5),
                    follow_redirects: true,
                }),
            )
            .await
            {
                Ok(Ok(resp)) if (200..300).contains(&resp.status) => {
                    println!("\u{2713} Network OK (example.com returned {})", resp.status);
                }
                Ok(Ok(resp)) => {
                    println!(
                        "\u{2717} Network issue (example.com returned {})",
                        resp.status
                    );
                    all_ok = false;
                }
                Ok(Err(e)) => {
                    println!("\u{2717} Network failed: {e:?}");
                    all_ok = false;
                }
                Err(_) => {
                    println!("\u{2717} Network timeout (5s)");
                    all_ok = false;
                }
            }

            if all_ok {
                println!("\nAll checks passed. Kaelo is healthy.");
            } else {
                println!("\nSome checks failed. See above for details.");
                std::process::exit(1);
            }
        }
    }

    Ok(())
}

async fn serve_http(server: kaelo::mcp::KaeloServer, host: &str, port: u16) -> anyhow::Result<()> {
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use hyper::body::Incoming;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    use std::convert::Infallible;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;
    use tower_service::Service;

    type BoxBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

    fn full_body(data: impl Into<Bytes>) -> BoxBody {
        Full::new(data.into()).map_err(|_| unreachable!()).boxed()
    }

    fn add_cors_headers<B>(mut response: Response<B>) -> Response<B> {
        let headers = response.headers_mut();
        headers.insert("access-control-allow-origin", "*".parse().unwrap());
        headers.insert(
            "access-control-allow-methods",
            "POST, GET, OPTIONS".parse().unwrap(),
        );
        headers.insert(
            "access-control-allow-headers",
            "Content-Type".parse().unwrap(),
        );
        response
    }

    fn cors_preflight() -> Response<BoxBody> {
        let mut resp = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(full_body(Bytes::new()))
            .unwrap();
        let headers = resp.headers_mut();
        headers.insert("access-control-allow-origin", "*".parse().unwrap());
        headers.insert(
            "access-control-allow-methods",
            "POST, GET, OPTIONS".parse().unwrap(),
        );
        headers.insert(
            "access-control-allow-headers",
            "Content-Type".parse().unwrap(),
        );
        headers.insert("access-control-max-age", "86400".parse().unwrap());
        resp
    }

    fn health_response() -> Response<BoxBody> {
        let body = full_body(Bytes::from(
            serde_json::json!({
                "status": "ok",
                "version": env!("CARGO_PKG_VERSION")
            })
            .to_string(),
        ));
        add_cors_headers(
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
    }

    fn not_found() -> Response<BoxBody> {
        add_cors_headers(
            Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(full_body(Bytes::from("not found")))
                .unwrap(),
        )
    }

    let ct = CancellationToken::new();
    let ct_child = ct.child_token();

    let allowed_hosts = vec![
        "localhost".into(),
        "127.0.0.1".into(),
        "::1".into(),
        host.to_string(),
        format!("{host}:{port}"),
    ];

    let mcp_config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true)
        .with_allowed_hosts(allowed_hosts)
        .with_cancellation_token(ct_child);

    let mcp_service: StreamableHttpService<kaelo::mcp::KaeloServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(LocalSessionManager::default()),
            mcp_config,
        );
    let mcp_service = Arc::new(tokio::sync::Mutex::new(mcp_service));

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Kaelo HTTP server listening on http://{addr}");
    tracing::info!("  GET  /health — health check");
    tracing::info!("  POST /mcp    — MCP JSON-RPC endpoint");

    let shutdown_ct = ct.clone();
    let shutdown_signal = tokio::signal::ctrl_c();

    tokio::pin!(shutdown_signal);

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                let (stream, _) = accept_result?;
                let mcp_service = mcp_service.clone();
                let io = hyper_util::rt::TokioIo::new(stream);

                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let mcp_service = mcp_service.clone();
                        async move {
                            let response: Response<BoxBody> = match (req.method(), req.uri().path()) {
                                (&Method::OPTIONS, _) => cors_preflight(),
                                (&Method::GET, "/health") => health_response(),
                                (&Method::POST, "/mcp") | (&Method::GET, "/mcp") | (&Method::DELETE, "/mcp") => {
                                    let mut svc = mcp_service.lock().await;
                                    let future = svc.call(req);
                                    drop(svc);
                                    let resp = future.await.map_err(|_| {
                                        std::io::Error::other("mcp service error")
                                    })?;
                                    add_cors_headers(resp)
                                }
                                _ => not_found(),
                            };
                            Ok::<_, std::io::Error>(response)
                        }
                    });

                    let _ = http1::Builder::new()
                        .serve_connection(io, service)
                        .await;
                });
            }
            _ = &mut shutdown_signal => {
                tracing::info!("Received Ctrl+C, shutting down HTTP server...");
                shutdown_ct.cancel();
                let cleanup = kaelo::fetch::backends::shutdown_browser_pool();
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), cleanup).await;
                tracing::info!("Shutdown complete.");
                break;
            }
        }
    }

    Ok(())
}
