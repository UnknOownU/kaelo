//! SearXNG JSON API search backend.

use anyhow::Result;
use serde::Deserialize;

use super::{SearchBackend, SearchResult};

pub struct SearXngBackend {
    base_url: String,
    client: reqwest::Client,
}

impl SearXngBackend {
    pub fn new(base_url: String) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self { base_url, client })
    }

    pub fn parse_response(json: &str) -> Result<Vec<SearXngItem>> {
        let response: SearXngResponse = serde_json::from_str(json)?;
        Ok(response.results)
    }
}

#[derive(Deserialize)]
struct SearXngResponse {
    results: Vec<SearXngItem>,
}

#[derive(Deserialize, Debug)]
pub struct SearXngItem {
    pub title: String,
    pub url: String,
    pub content: String,
}

impl SearchBackend for SearXngBackend {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let url = format!(
            "{}/search?q={}&format=json",
            self.base_url.trim_end_matches('/'),
            urlencoding(query)
        );

        let response = self.client.get(&url).send().await?;
        let body = response.text().await?;
        let items = Self::parse_response(&body)?;

        let results = items
            .into_iter()
            .take(max_results)
            .map(|item| SearchResult {
                title: item.title,
                url: item.url,
                description: item.content,
            })
            .collect();

        Ok(results)
    }
}

fn urlencoding(s: &str) -> String {
    let mut encoded = String::new();
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_searxng_json() {
        let json = r#"{
            "results": [
                {
                    "title": "Rust Programming Language",
                    "url": "https://www.rust-lang.org/",
                    "content": "A language empowering everyone to build reliable software."
                },
                {
                    "title": "Rust Docs",
                    "url": "https://doc.rust-lang.org/",
                    "content": "The Rust documentation."
                }
            ],
            "unresponsive_engines": []
        }"#;

        let items = SearXngBackend::parse_response(json).expect("parse");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Rust Programming Language");
        assert_eq!(items[0].url, "https://www.rust-lang.org/");
        assert_eq!(items[1].content, "The Rust documentation.");
    }

    #[test]
    fn parse_searxng_empty_results() {
        let json = r#"{"results": [], "unresponsive_engines": []}"#;
        let items = SearXngBackend::parse_response(json).expect("parse");
        assert!(items.is_empty());
    }

    #[test]
    fn parse_searxng_invalid_json() {
        let result = SearXngBackend::parse_response("not json");
        assert!(result.is_err());
    }

    #[test]
    fn searxng_search_converts_to_search_result() {
        let json = r#"{
            "results": [
                {
                    "title": "Test",
                    "url": "https://example.com",
                    "content": "Some description"
                }
            ]
        }"#;

        let items = SearXngBackend::parse_response(json).expect("parse");
        let results: Vec<SearchResult> = items
            .into_iter()
            .take(5)
            .map(|item| SearchResult {
                title: item.title,
                url: item.url,
                description: item.content,
            })
            .collect();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].description, "Some description");
    }
}
