//! Search abstraction layer with pluggable backends.
//!
//! Supports DuckDuckGo (default) and SearXNG (user-provided URL).

pub mod ddg;
pub mod searxng;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub description: String,
}

pub trait SearchBackend: Send + Sync {
    fn search(
        &self,
        query: &str,
        max_results: usize,
    ) -> impl std::future::Future<Output = Result<Vec<SearchResult>>> + Send;
}
