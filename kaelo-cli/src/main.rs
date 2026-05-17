use clap::{Parser, Subcommand};
use kaelo_core::update;
use kaelo_fetch::backend::FetchBackend;
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
    /// Start MCP server (stdio transport)
    Serve,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    tracing::info!("Kaelo v{} starting...", env!("CARGO_PKG_VERSION"));
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve => {
            let config = kaelo_core::config::Config::load();
            let storage = kaelo_core::storage::Storage::open(&config.db_path.to_string_lossy())?;
            let server = kaelo_mcp::KaeloServer::with_config(
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

                if kaelo_core::update::should_check(&cache_path) {
                    tokio::spawn(async move {
                        match kaelo_core::update::check_for_update().await {
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
                    // Write cache AFTER spawning (don't block on it)
                    if let Err(e) = kaelo_core::update::write_check_cache(&cache_path) {
                        tracing::debug!("Failed to write update cache: {e}");
                    }
                }
            }

            tokio::select! {
                result = kaelo_mcp::server::serve_stdio(server) => {
                    result?;
                }
                _ = tokio::signal::ctrl_c() => {
                    tracing::info!("Received Ctrl+C, shutting down gracefully...");
                    let cleanup = async {
                        kaelo_fetch::backends::shutdown_browser_pool().await;
                    };
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        cleanup,
                    ).await;
                    tracing::info!("Shutdown complete.");
                }
            }
        }
        Commands::Cache { command } => {
            let config = kaelo_core::config::Config::load();
            match command {
                CacheCommands::Status => {
                    match kaelo_core::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo_core::storage::route_cache::RouteCache::new(&storage);
                            let content_cache =
                                kaelo_core::storage::content_cache::ContentCache::new(&storage);

                            let route_count = route_cache.count_entries().unwrap_or(0);
                            let domains = route_cache.get_all_domains().unwrap_or_default();
                            let stats = content_cache.stats().unwrap_or_else(|_| {
                                kaelo_core::storage::content_cache::CacheStats {
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
                    match kaelo_core::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let content_cache =
                                kaelo_core::storage::content_cache::ContentCache::new(&storage);
                            content_cache.clear_all()?;
                            println!("Cache cleared.");
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::Export { path } => {
                    match kaelo_core::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo_core::storage::route_cache::RouteCache::new(&storage);
                            route_cache.export_json(&path)?;
                            println!("Exported route cache to {}", path.display());
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::Import { path } => {
                    match kaelo_core::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo_core::storage::route_cache::RouteCache::new(&storage);
                            let count = route_cache.import_json(&path)?;
                            println!("Imported {} strategies from {}", count, path.display());
                        }
                        Err(e) => eprintln!("Error opening database: {}", e),
                    }
                }
                CacheCommands::ImportPack { path } => {
                    match kaelo_core::storage::Storage::open(&config.db_path.to_string_lossy()) {
                        Ok(storage) => {
                            let route_cache =
                                kaelo_core::storage::route_cache::RouteCache::new(&storage);
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
            let backend = kaelo_fetch::backends::HttpSimple::new()
                .map_err(|e| anyhow::anyhow!("Failed to create backend: {}", e))?;

            let request = kaelo_core::types::FetchRequest {
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
            let extracted = kaelo_core::extraction::extract(&html, &url, &response.content_type)?;

            match extracted {
                Some(content) => print!("{}", content.text_content),
                None => print!("{}", html),
            }
        }
        Commands::Setup => {
            let config = kaelo_core::config::Config::load();
            println!("Kaelo Configuration:");
            println!("  Cache enabled: {}", config.cache_enabled);
            println!("  Cache max size: {} bytes", config.cache_max_size);
            println!("  Cache TTL: {} seconds", config.cache_ttl);
            println!("  Database: {:?}", config.db_path);
            println!("  Log level: {}", config.log_level);
            println!();
            println!("MCP Server Configuration (for OpenCode/Claude Code):");
            println!("  Add to your MCP config:");
            println!("  {{");
            println!("    \"mcpServers\": {{");
            println!("      \"kaelo\": {{");
            println!("        \"command\": \"kaelo\",");
            println!("        \"args\": [\"serve\"]");
            println!("      }}");
            println!("    }}");
            println!("  }}");
            println!();
            println!("Environment Variables:");
            println!("  KAELO_CACHE_ENABLED   - Enable/disable cache (default: true)");
            println!("  KAELO_CACHE_MAX_SIZE  - Max cache size in bytes (default: 52428800)");
            println!("  KAELO_CACHE_TTL       - Cache TTL in seconds (default: 3600)");
            println!("  KAELO_DB_PATH         - Database path (default: ~/.config/kaelo/kaelo.db)");
            println!("  KAELO_LOG_LEVEL       - Log level (default: info)");
        }
        Commands::ProveIt { challenge } => {
            println!("Kaelo prove-it: Verifying installation...\n");

            let backend = kaelo_fetch::backends::HttpSimple::new()
                .map_err(|e| anyhow::anyhow!("Backend failed: {}", e))?;
            println!("✓ HTTP backend initialized (HttpSimple)");

            let (demo_url, label) = if challenge {
                println!("Challenge mode: attempting a harder, Cloudflare-protected URL...");
                ("https://nowsecure.nl", "nowsecure.nl (Cloudflare)")
            } else {
                ("https://httpbin.org/get", "httpbin.org")
            };

            print!("→ Fetching {}... ", label);

            let request = kaelo_core::types::FetchRequest {
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
            match kaelo_core::extraction::extract(html, "https://example.com", "text/html") {
                Ok(Some(content)) => {
                    println!("✓ (extracted {} chars)", content.text_content.len());
                }
                Ok(None) => println!("✗ No content extracted"),
                Err(e) => println!("✗ {}", e),
            }

            print!("→ Testing storage... ");
            match kaelo_core::storage::Storage::open(":memory:") {
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
                    println!("Run: cargo install kaelo-cli");
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
    }

    Ok(())
}
