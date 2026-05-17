//! DuckDuckGo HTML search backend.

use anyhow::Result;

use super::{SearchBackend, SearchResult};

pub struct DuckDuckGoBackend {
    client: reqwest::Client,
}

impl DuckDuckGoBackend {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (compatible; Kaelo/0.1)")
            .build()?;
        Ok(Self { client })
    }

    pub fn parse_results(html: &str, max: usize) -> Vec<SearchResult> {
        let mut results = Vec::new();
        let mut pos = 0;

        while results.len() < max {
            let result_start = match html[pos..].find("class=\"result__a\"") {
                Some(i) => pos + i,
                None => break,
            };

            let title = extract_tag_content(html, result_start);

            let href_start = match html[..result_start].rfind("href=\"") {
                Some(i) => i + 6,
                None => {
                    pos = result_start + 16;
                    continue;
                }
            };

            let href_end = match html[href_start..].find('"') {
                Some(i) => href_start + i,
                None => {
                    pos = result_start + 16;
                    continue;
                }
            };

            let href = html[href_start..href_end].to_string();
            let url = clean_ddg_url(&href);

            if url.starts_with("http") {
                results.push(SearchResult {
                    title,
                    url,
                    description: String::new(),
                });
            }

            pos = result_start + 16;
        }

        results
    }
}

impl SearchBackend for DuckDuckGoBackend {
    async fn search(&self, query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding(query));

        let response = self.client.get(&url).send().await?;
        let html = response.text().await?;

        Ok(Self::parse_results(&html, max_results))
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

fn extract_tag_content(html: &str, tag_pos: usize) -> String {
    let after_tag = match html[tag_pos..].find('>') {
        Some(i) => tag_pos + i + 1,
        None => return String::new(),
    };

    let end = match html[after_tag..].find('<') {
        Some(i) => after_tag + i,
        None => return html[after_tag..].to_string(),
    };

    html[after_tag..end]
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .trim()
        .to_string()
}

fn clean_ddg_url(url: &str) -> String {
    if let Some(start) = url.find("uddg=") {
        let encoded = &url[start + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        let encoded_portion = &encoded[..end];
        match percent_decode(encoded_portion) {
            Some(decoded) => return decoded,
            None => return encoded_portion.to_string(),
        }
    }
    url.strip_prefix("//")
        .map(|s| format!("https://{s}"))
        .unwrap_or_else(|| url.to_string())
}

fn percent_decode(s: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hi = chars.next()?.to_digit(16)? as u8;
            let lo = chars.next()?.to_digit(16)? as u8;
            result.push(char::from((hi << 4) | lo));
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ddg_results_from_html() {
        let html = r#"
        <div class="result">
            <a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage" class="result__a">Example Page</a>
        </div>
        <div class="result">
            <a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftest.org%2Fdocs" class="result__a">Test Docs</a>
        </div>
        "#;

        let results = DuckDuckGoBackend::parse_results(html, 5);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Example Page");
        assert_eq!(results[0].url, "https://example.com/page");
        assert_eq!(results[1].title, "Test Docs");
        assert_eq!(results[1].url, "https://test.org/docs");
    }

    #[test]
    fn parse_ddg_results_max_limit() {
        let html = r#"
        <div><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.com" class="result__a">A</a></div>
        <div><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fb.com" class="result__a">B</a></div>
        <div><a href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fc.com" class="result__a">C</a></div>
        "#;

        let results = DuckDuckGoBackend::parse_results(html, 2);
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn parse_ddg_results_empty_html() {
        let results = DuckDuckGoBackend::parse_results("<html><body></body></html>", 5);
        assert!(results.is_empty());
    }

    #[test]
    fn clean_ddg_url_strips_redirect() {
        let url = "//duckduckgo.com/l/?uddg=https%3A%2F%2Freal.site%2Fpath&t=h_";
        let cleaned = clean_ddg_url(url);
        assert_eq!(cleaned, "https://real.site/path");
    }

    #[test]
    fn clean_ddg_url_plain_url() {
        let url = "https://example.com/direct";
        let cleaned = clean_ddg_url(url);
        assert_eq!(cleaned, "https://example.com/direct");
    }

    #[test]
    fn clean_ddg_url_protocol_relative() {
        let url = "//cdn.example.com/resource";
        let cleaned = clean_ddg_url(url);
        assert_eq!(cleaned, "https://cdn.example.com/resource");
    }

    #[test]
    fn urlencoding_spaces_and_special() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("rust & cargo"), "rust+%26+cargo");
        assert_eq!(urlencoding("abc123-_."), "abc123-_.");
    }

    #[test]
    #[ignore]
    fn ddg_live_search() {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let backend = DuckDuckGoBackend::new().expect("backend creation");
        let results = rt
            .block_on(backend.search("rust programming language", 3))
            .expect("search");
        assert!(
            !results.is_empty(),
            "should return results for a common query"
        );
        assert!(results[0].url.starts_with("http"), "URL should be absolute");
    }
}
