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
}
