use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct FetchRequest {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub timeout: Duration,
    pub follow_redirects: bool,
}

#[derive(Debug, Clone)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub content_type: String,
    pub latency: Duration,
}

#[derive(Debug, Clone)]
pub enum FetchError {
    Timeout,
    HttpError(u16),
    TlsError(String),
    BrowserError(String),
    NetworkError(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Timeout => write!(f, "request timed out"),
            FetchError::HttpError(status) => write!(f, "HTTP error: {status}"),
            FetchError::TlsError(msg) => write!(f, "TLS error: {msg}"),
            FetchError::BrowserError(msg) => write!(f, "browser error: {msg}"),
            FetchError::NetworkError(msg) => write!(f, "network error: {msg}"),
        }
    }
}

impl std::error::Error for FetchError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Strategy {
    HttpSimple,
    TlsChrome,
    TlsMobile,
    Headless,
    PublicApi,
    YouTube,
}

// === v1.1.0 New Types ===

#[derive(Default, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExtractionMode {
    #[default]
    Markdown,
    Text,
    Html,
    Links,
    JsonLd,
    Tables,
    Metadata,
}

impl ExtractionMode {
    pub fn parse_mode(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "text" => ExtractionMode::Text,
            "html" => ExtractionMode::Html,
            "links" => ExtractionMode::Links,
            "json-ld" | "jsonld" => ExtractionMode::JsonLd,
            "tables" => ExtractionMode::Tables,
            "metadata" => ExtractionMode::Metadata,
            _ => ExtractionMode::Markdown,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ExtractionMode::Markdown => "markdown",
            ExtractionMode::Text => "text",
            ExtractionMode::Html => "html",
            ExtractionMode::Links => "links",
            ExtractionMode::JsonLd => "json-ld",
            ExtractionMode::Tables => "tables",
            ExtractionMode::Metadata => "metadata",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BlockType {
    Cloudflare,
    PerimeterX,
    DataDome,
    Akamai,
    Captcha,
    RateLimit,
    AuthWall,
    Paywall,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FetchDiagnostics {
    pub strategy_used: String,
    pub cache_hit: bool,
    pub spa_detected: bool,
    pub js_rendered: bool,
    pub redirect_count: u32,
    pub content_length: usize,
    pub line_count: usize,
    pub token_count: usize,
    pub token_budget: Option<usize>,
    pub confidence: Confidence,
    pub extraction_mode: String,
    pub fetch_latency_ms: u64,
    pub block_detected: Option<BlockType>,
}

impl FetchDiagnostics {
    pub fn to_meta(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    pub fn summary(&self) -> String {
        let cache = if self.cache_hit {
            "cache HIT"
        } else {
            "cache MISS"
        };
        let mut parts = vec![
            format!("{}", self.strategy_used),
            format!("{}ms", self.fetch_latency_ms),
            cache.to_string(),
            self.extraction_mode.clone(),
            format!("{} chars", self.content_length),
        ];
        if self.spa_detected {
            parts.push("SPA detected".to_string());
        }
        if let Some(ref block) = self.block_detected {
            parts.push(format!("blocked:{:?}", block));
        }
        parts.join(", ")
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Citation {
    pub text: String,
    pub source_url: String,
    pub context: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Evidence {
    pub url: String,
    pub retrieved_at: String,
    pub content_hash: String,
    pub strategy: String,
    pub confidence: Confidence,
    pub citations: Vec<Citation>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageDiff {
    pub url: String,
    pub old_hash: Option<String>,
    pub new_hash: String,
    pub added_lines: u32,
    pub removed_lines: u32,
    pub unchanged_lines: u32,
    pub similarity_percent: f64,
    pub diff_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extraction_mode_default() {
        assert_eq!(ExtractionMode::default(), ExtractionMode::Markdown);
    }

    #[test]
    fn test_extraction_mode_from_str() {
        assert_eq!(
            ExtractionMode::parse_mode("markdown"),
            ExtractionMode::Markdown
        );
        assert_eq!(ExtractionMode::parse_mode("TEXT"), ExtractionMode::Text);
        assert_eq!(ExtractionMode::parse_mode("links"), ExtractionMode::Links);
        assert_eq!(
            ExtractionMode::parse_mode("json-ld"),
            ExtractionMode::JsonLd
        );
        assert_eq!(
            ExtractionMode::parse_mode("invalid"),
            ExtractionMode::Markdown
        );
    }

    #[test]
    fn test_extraction_mode_as_str() {
        assert_eq!(ExtractionMode::JsonLd.as_str(), "json-ld");
        assert_eq!(ExtractionMode::Markdown.as_str(), "markdown");
    }

    #[test]
    fn test_diagnostics_summary() {
        let d = FetchDiagnostics {
            strategy_used: "HttpSimple".to_string(),
            cache_hit: false,
            spa_detected: true,
            js_rendered: false,
            redirect_count: 1,
            content_length: 12400,
            line_count: 340,
            token_count: 8000,
            token_budget: Some(10000),
            confidence: Confidence::High,
            extraction_mode: "markdown".to_string(),
            fetch_latency_ms: 42,
            block_detected: None,
        };
        let s = d.summary();
        assert!(s.contains("HttpSimple"));
        assert!(s.contains("42ms"));
        assert!(s.contains("SPA detected"));
    }

    #[test]
    fn test_diagnostics_to_meta() {
        let d = FetchDiagnostics {
            strategy_used: "HttpSimple".to_string(),
            cache_hit: true,
            spa_detected: false,
            js_rendered: false,
            redirect_count: 0,
            content_length: 100,
            line_count: 10,
            token_count: 50,
            token_budget: None,
            confidence: Confidence::High,
            extraction_mode: "markdown".to_string(),
            fetch_latency_ms: 10,
            block_detected: None,
        };
        let meta = d.to_meta();
        assert_eq!(meta["strategy_used"], "HttpSimple");
        assert_eq!(meta["cache_hit"], true);
    }
}
