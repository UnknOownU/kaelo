use std::collections::HashMap;
use std::env;
use std::fmt;
use std::path::PathBuf;

#[derive(Clone)]
pub struct AuthConfig {
    pub bearer_token: Option<String>,
    pub basic_auth: Option<(String, String)>,
    pub headers: HashMap<String, String>,
}

impl fmt::Debug for AuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthConfig")
            .field(
                "bearer_token",
                &self.bearer_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "basic_auth",
                &self.basic_auth.as_ref().map(|_| "[REDACTED]"),
            )
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub cache_enabled: bool,
    pub cache_max_size: u64,
    pub cache_max_entry: u64,
    pub cache_ttl: u64,
    pub cache_compression: String,
    pub db_path: PathBuf,
    pub log_level: String,
    pub per_domain_auth: HashMap<String, AuthConfig>,
    pub searxng_url: Option<String>,
    pub search_backend: Option<String>,
    pub default_strategy: Option<String>,
    pub allow_private_networks: bool,
    pub respect_robots_txt: bool,
    pub default_extraction_mode: String,
    pub http_server_enabled: bool,
    pub http_server_port: u16,
    pub http_server_host: String,
}

fn parse_bool(val: &str) -> bool {
    matches!(val, "true" | "1")
}

fn default_db_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("kaelo")
        .join("kaelo.db")
}

fn default_config_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("kaelo")
        .join("config.toml")
}

fn parse_auth_from_env() -> HashMap<String, AuthConfig> {
    let mut auth_map: HashMap<String, AuthConfig> = HashMap::new();

    for (key, value) in env::vars() {
        let rest = match key.strip_prefix("KAELO_AUTH_") {
            Some(r) => r,
            None => continue,
        };

        // KAELO_AUTH_GITHUB_COM → github.com
        let domain = rest.to_lowercase().replace('_', ".");

        let config = auth_map
            .entry(domain.clone())
            .or_insert_with(|| AuthConfig {
                bearer_token: None,
                basic_auth: None,
                headers: HashMap::new(),
            });

        if let Some(token) = value.strip_prefix("Bearer:") {
            config.bearer_token = Some(token.to_string());
        } else if let Some(creds) = value.strip_prefix("Basic:") {
            let parts: Vec<&str> = creds.splitn(2, ':').collect();
            if parts.len() == 2 {
                config.basic_auth = Some((parts[0].to_string(), parts[1].to_string()));
            }
        } else if let Some(header_val) = value.strip_prefix("Header:") {
            let parts: Vec<&str> = header_val.splitn(2, ':').collect();
            if parts.len() == 2 {
                config
                    .headers
                    .insert(parts[0].to_string(), parts[1].to_string());
            }
        }
    }

    auth_map
}

impl Config {
    pub fn from_env() -> Self {
        Config {
            cache_enabled: env::var("KAELO_CACHE_ENABLED")
                .map(|v| parse_bool(&v))
                .unwrap_or(true),
            cache_max_size: env::var("KAELO_CACHE_MAX_SIZE")
                .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
                .unwrap_or(52_428_800),
            cache_max_entry: env::var("KAELO_CACHE_MAX_ENTRY")
                .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
                .unwrap_or(102_400),
            cache_ttl: env::var("KAELO_CACHE_TTL")
                .and_then(|v| v.parse::<u64>().map_err(|_| std::env::VarError::NotPresent))
                .unwrap_or(3600),
            cache_compression: env::var("KAELO_CACHE_COMPRESSION")
                .unwrap_or_else(|_| "gzip".to_string()),
            db_path: env::var("KAELO_DB_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_db_path()),
            log_level: env::var("KAELO_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
            per_domain_auth: parse_auth_from_env(),
            searxng_url: env::var("KAELO_SEARXNG_URL").ok(),
            search_backend: env::var("KAELO_SEARCH_BACKEND").ok(),
            default_strategy: env::var("KAELO_DEFAULT_STRATEGY").ok(),
            allow_private_networks: env::var("KAELO_ALLOW_PRIVATE_NETWORKS")
                .map(|v| parse_bool(&v))
                .unwrap_or(false),
            respect_robots_txt: env::var("KAELO_RESPECT_ROBOTS_TXT")
                .map(|v| parse_bool(&v))
                .unwrap_or(false),
            default_extraction_mode: env::var("KAELO_DEFAULT_EXTRACTION_MODE")
                .unwrap_or_else(|_| "markdown".to_string()),
            http_server_enabled: env::var("KAELO_HTTP_SERVER_ENABLED")
                .map(|v| parse_bool(&v))
                .unwrap_or(false),
            http_server_port: env::var("KAELO_HTTP_SERVER_PORT")
                .and_then(|v| v.parse::<u16>().map_err(|_| std::env::VarError::NotPresent))
                .unwrap_or(8080),
            http_server_host: env::var("KAELO_HTTP_SERVER_HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
        }
    }

    /// Priority: env vars > config file > defaults.
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let value: toml::Value = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse TOML config: {}", e))?;

        let mut config = Config {
            cache_enabled: true,
            cache_max_size: 52_428_800,
            cache_max_entry: 102_400,
            cache_ttl: 3600,
            cache_compression: "gzip".to_string(),
            db_path: default_db_path(),
            log_level: "info".to_string(),
            per_domain_auth: HashMap::new(),
            searxng_url: None,
            search_backend: None,
            default_strategy: None,
            allow_private_networks: false,
            respect_robots_txt: false,
            default_extraction_mode: "markdown".to_string(),
            http_server_enabled: false,
            http_server_port: 8080,
            http_server_host: "127.0.0.1".to_string(),
        };

        if let Some(v) = value.get("cache_enabled").and_then(|v| v.as_bool()) {
            config.cache_enabled = v;
        }
        if let Some(v) = value.get("cache_max_size").and_then(|v| v.as_integer()) {
            config.cache_max_size = v as u64;
        }
        if let Some(v) = value.get("cache_max_entry").and_then(|v| v.as_integer()) {
            config.cache_max_entry = v as u64;
        }
        if let Some(v) = value.get("cache_ttl").and_then(|v| v.as_integer()) {
            config.cache_ttl = v as u64;
        }
        if let Some(v) = value.get("cache_compression").and_then(|v| v.as_str()) {
            config.cache_compression = v.to_string();
        }
        if let Some(v) = value.get("db_path").and_then(|v| v.as_str()) {
            config.db_path = PathBuf::from(v);
        }
        if let Some(v) = value.get("log_level").and_then(|v| v.as_str()) {
            config.log_level = v.to_string();
        }
        if let Some(v) = value.get("searxng_url").and_then(|v| v.as_str()) {
            config.searxng_url = Some(v.to_string());
        }
        if let Some(v) = value.get("search_backend").and_then(|v| v.as_str()) {
            config.search_backend = Some(v.to_string());
        }
        if let Some(v) = value.get("default_strategy").and_then(|v| v.as_str()) {
            config.default_strategy = Some(v.to_string());
        }
        if let Some(v) = value
            .get("allow_private_networks")
            .and_then(|v| v.as_bool())
        {
            config.allow_private_networks = v;
        }
        if let Some(v) = value.get("respect_robots_txt").and_then(|v| v.as_bool()) {
            config.respect_robots_txt = v;
        }
        if let Some(v) = value
            .get("default_extraction_mode")
            .and_then(|v| v.as_str())
        {
            config.default_extraction_mode = v.to_string();
        }
        if let Some(v) = value.get("http_server_enabled").and_then(|v| v.as_bool()) {
            config.http_server_enabled = v;
        }
        if let Some(v) = value.get("http_server_port").and_then(|v| v.as_integer()) {
            config.http_server_port = v as u16;
        }
        if let Some(v) = value.get("http_server_host").and_then(|v| v.as_str()) {
            config.http_server_host = v.to_string();
        }

        if let Ok(v) = env::var("KAELO_CACHE_ENABLED") {
            config.cache_enabled = parse_bool(&v);
        }
        if let Ok(v) = env::var("KAELO_CACHE_MAX_SIZE") {
            if let Ok(parsed) = v.parse::<u64>() {
                config.cache_max_size = parsed;
            }
        }
        if let Ok(v) = env::var("KAELO_CACHE_MAX_ENTRY") {
            if let Ok(parsed) = v.parse::<u64>() {
                config.cache_max_entry = parsed;
            }
        }
        if let Ok(v) = env::var("KAELO_CACHE_TTL") {
            if let Ok(parsed) = v.parse::<u64>() {
                config.cache_ttl = parsed;
            }
        }
        if let Ok(v) = env::var("KAELO_CACHE_COMPRESSION") {
            config.cache_compression = v;
        }
        if let Ok(v) = env::var("KAELO_DB_PATH") {
            config.db_path = PathBuf::from(v);
        }
        if let Ok(v) = env::var("KAELO_LOG_LEVEL") {
            config.log_level = v;
        }
        if let Ok(v) = env::var("KAELO_SEARXNG_URL") {
            config.searxng_url = Some(v);
        }
        if let Ok(v) = env::var("KAELO_SEARCH_BACKEND") {
            config.search_backend = Some(v);
        }
        if let Ok(v) = env::var("KAELO_DEFAULT_STRATEGY") {
            config.default_strategy = Some(v);
        }
        if let Ok(v) = env::var("KAELO_ALLOW_PRIVATE_NETWORKS") {
            config.allow_private_networks = parse_bool(&v);
        }
        if let Ok(v) = env::var("KAELO_RESPECT_ROBOTS_TXT") {
            config.respect_robots_txt = parse_bool(&v);
        }
        if let Ok(v) = env::var("KAELO_DEFAULT_EXTRACTION_MODE") {
            config.default_extraction_mode = v;
        }
        if let Ok(v) = env::var("KAELO_HTTP_SERVER_ENABLED") {
            config.http_server_enabled = parse_bool(&v);
        }
        if let Ok(v) = env::var("KAELO_HTTP_SERVER_PORT") {
            if let Ok(parsed) = v.parse::<u16>() {
                config.http_server_port = parsed;
            }
        }
        if let Ok(v) = env::var("KAELO_HTTP_SERVER_HOST") {
            config.http_server_host = v;
        }
        let env_auth = parse_auth_from_env();
        if !env_auth.is_empty() {
            config.per_domain_auth = env_auth;
        }

        Ok(config)
    }

    /// Falls back to `from_env()` if no config file exists.
    pub fn load() -> Self {
        let config_path = default_config_path();
        if config_path.exists() {
            Self::from_file(&config_path).unwrap_or_else(|_| Self::from_env())
        } else {
            Self::from_env()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.cache_max_size == 0 {
            return Err("cache_max_size must be > 0".to_string());
        }
        if self.cache_max_entry == 0 {
            return Err("cache_max_entry must be > 0".to_string());
        }
        if self.cache_max_entry > self.cache_max_size {
            return Err("cache_max_entry must be <= cache_max_size".to_string());
        }
        if self.cache_ttl == 0 {
            return Err("cache_ttl must be > 0".to_string());
        }
        if !matches!(self.cache_compression.as_str(), "gzip" | "none") {
            return Err("cache_compression must be \"gzip\" or \"none\"".to_string());
        }
        if !matches!(
            self.log_level.as_str(),
            "trace" | "debug" | "info" | "warn" | "error"
        ) {
            return Err("log_level must be one of: trace, debug, info, warn, error".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unset_kaelo_vars() {
        for (key, _) in env::vars() {
            if key.starts_with("KAELO_") {
                env::remove_var(key);
            }
        }
    }

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_default_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        let config = Config::from_env();
        assert!(config.cache_enabled);
        assert_eq!(config.cache_max_size, 52_428_800);
        assert_eq!(config.cache_max_entry, 102_400);
        assert_eq!(config.cache_ttl, 3600);
        assert_eq!(config.cache_compression, "gzip");
        assert_eq!(config.log_level, "info");
        assert!(config.db_path.ends_with(".config/kaelo/kaelo.db"));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_custom_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_CACHE_ENABLED", "false");
        env::set_var("KAELO_CACHE_MAX_SIZE", "1000");
        env::set_var("KAELO_CACHE_MAX_ENTRY", "500");
        env::set_var("KAELO_CACHE_TTL", "7200");
        env::set_var("KAELO_CACHE_COMPRESSION", "none");
        env::set_var("KAELO_DB_PATH", "/tmp/test.db");
        env::set_var("KAELO_LOG_LEVEL", "debug");

        let config = Config::from_env();
        assert!(!config.cache_enabled);
        assert_eq!(config.cache_max_size, 1000);
        assert_eq!(config.cache_max_entry, 500);
        assert_eq!(config.cache_ttl, 7200);
        assert_eq!(config.cache_compression, "none");
        assert_eq!(config.db_path, PathBuf::from("/tmp/test.db"));
        assert_eq!(config.log_level, "debug");
        assert!(config.validate().is_ok());

        unset_kaelo_vars();
    }

    #[test]
    fn test_validation_rejects_invalid() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();

        // cache_max_size = 0
        let mut config = Config::from_env();
        config.cache_max_size = 0;
        assert!(config.validate().is_err());

        // cache_max_entry = 0
        config.cache_max_size = 1000;
        config.cache_max_entry = 0;
        assert!(config.validate().is_err());

        // cache_max_entry > cache_max_size
        config.cache_max_entry = 2000;
        assert!(config.validate().is_err());

        // cache_ttl = 0
        config.cache_max_entry = 500;
        config.cache_ttl = 0;
        assert!(config.validate().is_err());

        // invalid compression
        config.cache_ttl = 3600;
        config.cache_compression = "brotli".to_string();
        assert!(config.validate().is_err());

        // invalid log_level
        config.cache_compression = "gzip".to_string();
        config.log_level = "verbose".to_string();
        assert!(config.validate().is_err());

        unset_kaelo_vars();
    }

    #[test]
    fn test_bool_parsing_variants() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_CACHE_ENABLED", "1");
        assert!(Config::from_env().cache_enabled);

        env::set_var("KAELO_CACHE_ENABLED", "0");
        assert!(!Config::from_env().cache_enabled);

        env::set_var("KAELO_CACHE_ENABLED", "true");
        assert!(Config::from_env().cache_enabled);

        env::set_var("KAELO_CACHE_ENABLED", "false");
        assert!(!Config::from_env().cache_enabled);

        env::set_var("KAELO_CACHE_ENABLED", "yes");
        assert!(!Config::from_env().cache_enabled);

        unset_kaelo_vars();
    }

    #[test]
    fn test_auth_bearer_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_AUTH_GITHUB_COM", "Bearer:tok_abc123");

        let config = Config::from_env();
        assert!(config.per_domain_auth.contains_key("github.com"));
        let auth = &config.per_domain_auth["github.com"];
        assert_eq!(auth.bearer_token, Some("tok_abc123".to_string()));

        unset_kaelo_vars();
    }

    #[test]
    fn test_auth_basic_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_AUTH_API_EXAMPLE_COM", "Basic:user:pass123");

        let config = Config::from_env();
        assert!(config.per_domain_auth.contains_key("api.example.com"));
        let auth = &config.per_domain_auth["api.example.com"];
        assert_eq!(
            auth.basic_auth,
            Some(("user".to_string(), "pass123".to_string()))
        );

        unset_kaelo_vars();
    }

    #[test]
    fn test_auth_header_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_AUTH_FOO_COM", "Header:X-API-Key:mykey123");

        let config = Config::from_env();
        let auth = &config.per_domain_auth["foo.com"];
        assert_eq!(auth.headers.get("X-API-Key"), Some(&"mykey123".to_string()));

        unset_kaelo_vars();
    }

    #[test]
    fn test_auth_debug_redacted() {
        let config = AuthConfig {
            bearer_token: Some("secret_token".to_string()),
            basic_auth: Some(("user".to_string(), "pass".to_string())),
            headers: HashMap::new(),
        };
        let debug_str = format!("{:?}", config);
        assert!(!debug_str.contains("secret_token"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn test_from_file_overrides_defaults() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        let path = std::env::temp_dir().join("kaelo-test-from-file.toml");
        std::fs::write(&path, "cache_max_size = 999999\ncache_ttl = 7200").unwrap();
        let config = Config::from_file(&path).unwrap();
        assert_eq!(config.cache_max_size, 999999);
        assert_eq!(config.cache_ttl, 7200);
        assert_eq!(config.cache_max_entry, 102_400);
        std::fs::remove_file(path).ok();
        unset_kaelo_vars();
    }

    #[test]
    fn test_env_overrides_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_CACHE_MAX_SIZE", "12345678");
        let path = std::env::temp_dir().join("kaelo-test-env-override.toml");
        std::fs::write(&path, "cache_max_size = 999999").unwrap();
        let config = Config::from_file(&path).unwrap();
        assert_eq!(config.cache_max_size, 12345678);
        std::fs::remove_file(path).ok();
        unset_kaelo_vars();
    }

    #[test]
    fn test_missing_file_returns_error() {
        let config = Config::from_file(std::path::Path::new("/nonexistent/config.toml"));
        assert!(config.is_err());
    }

    #[test]
    fn test_new_fields_default_to_none() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        let config = Config::from_env();
        assert_eq!(config.searxng_url, None);
        assert_eq!(config.search_backend, None);
        assert_eq!(config.default_strategy, None);
        assert!(!config.allow_private_networks);
        assert!(!config.respect_robots_txt);
        assert_eq!(config.default_extraction_mode, "markdown");
        assert!(!config.http_server_enabled);
        assert_eq!(config.http_server_port, 8080);
        assert_eq!(config.http_server_host, "127.0.0.1");
        unset_kaelo_vars();
    }

    #[test]
    fn test_new_fields_from_env() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_SEARXNG_URL", "https://search.example.com");
        env::set_var("KAELO_SEARCH_BACKEND", "searxng");
        env::set_var("KAELO_DEFAULT_STRATEGY", "tls_mobile");
        env::set_var("KAELO_ALLOW_PRIVATE_NETWORKS", "true");
        env::set_var("KAELO_RESPECT_ROBOTS_TXT", "1");
        env::set_var("KAELO_DEFAULT_EXTRACTION_MODE", "html");
        env::set_var("KAELO_HTTP_SERVER_ENABLED", "true");
        env::set_var("KAELO_HTTP_SERVER_PORT", "9090");
        env::set_var("KAELO_HTTP_SERVER_HOST", "0.0.0.0");
        let config = Config::from_env();
        assert_eq!(
            config.searxng_url,
            Some("https://search.example.com".to_string())
        );
        assert_eq!(config.search_backend, Some("searxng".to_string()));
        assert_eq!(config.default_strategy, Some("tls_mobile".to_string()));
        assert!(config.allow_private_networks);
        assert!(config.respect_robots_txt);
        assert_eq!(config.default_extraction_mode, "html");
        assert!(config.http_server_enabled);
        assert_eq!(config.http_server_port, 9090);
        assert_eq!(config.http_server_host, "0.0.0.0");
        unset_kaelo_vars();
    }

    #[test]
    fn test_new_fields_from_toml() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        let path = std::env::temp_dir().join("kaelo-test-new-fields.toml");
        std::fs::write(
            &path,
            "searxng_url = \"https://my.searx.com\"\nsearch_backend = \"duckduckgo\"\ndefault_strategy = \"http\"\nallow_private_networks = true\nrespect_robots_txt = true\ndefault_extraction_mode = \"html\"\nhttp_server_enabled = true\nhttp_server_port = 9090\nhttp_server_host = \"0.0.0.0\"",
        )
        .unwrap();
        let config = Config::from_file(&path).unwrap();
        assert_eq!(config.searxng_url, Some("https://my.searx.com".to_string()));
        assert_eq!(config.search_backend, Some("duckduckgo".to_string()));
        assert_eq!(config.default_strategy, Some("http".to_string()));
        assert!(config.allow_private_networks);
        assert!(config.respect_robots_txt);
        assert_eq!(config.default_extraction_mode, "html");
        assert!(config.http_server_enabled);
        assert_eq!(config.http_server_port, 9090);
        assert_eq!(config.http_server_host, "0.0.0.0");
        std::fs::remove_file(path).ok();
        unset_kaelo_vars();
    }

    #[test]
    fn test_new_fields_env_overrides_toml() {
        let _lock = ENV_LOCK.lock().unwrap();
        unset_kaelo_vars();
        env::set_var("KAELO_SEARXNG_URL", "https://env.example.com");
        let path = std::env::temp_dir().join("kaelo-test-env-toml.toml");
        std::fs::write(&path, "searxng_url = \"https://file.example.com\"").unwrap();
        let config = Config::from_file(&path).unwrap();
        assert_eq!(
            config.searxng_url,
            Some("https://env.example.com".to_string())
        );
        std::fs::remove_file(path).ok();
        unset_kaelo_vars();
    }
}
