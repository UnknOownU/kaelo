use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use crate::fetch::backend::FetchBackend;

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{byte:02X}"));
            }
        }
    }
    out
}

fn extract_host(url: &str) -> String {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .split(':')
        .next()
        .unwrap_or(without_scheme)
        .to_lowercase()
}

fn extract_path(url: &str) -> &str {
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    without_scheme
        .find('/')
        .map_or("/", |i| &without_scheme[i..])
}

fn extract_query(url: &str) -> Option<&str> {
    let path = extract_path(url);
    path.find('?').map(|i| &path[i + 1..])
}

pub struct PublicApiBackend {
    client: reqwest::Client,
}

impl PublicApiBackend {
    pub fn new() -> Result<Self, FetchError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| FetchError::NetworkError(e.to_string()))?;
        Ok(Self { client })
    }
}

impl FetchBackend for PublicApiBackend {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let start = Instant::now();
        let url = &request.url;

        let api_url = if is_reddit_url(url) {
            build_reddit_api_url(url)?
        } else if is_hn_url(url) {
            build_hn_api_url(url)?
        } else if is_youtube_url(url) {
            build_youtube_api_url(url)?
        } else {
            return Err(FetchError::NetworkError(format!(
                "Unsupported URL for PublicApi: {url}"
            )));
        };

        let resp = self
            .client
            .get(&api_url)
            .header("User-Agent", "kaelo/0.1")
            .timeout(request.timeout)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    FetchError::Timeout
                } else {
                    FetchError::NetworkError(e.to_string())
                }
            })?;

        let status = resp.status().as_u16();
        if status >= 400 {
            return Err(FetchError::HttpError(status));
        }

        let json_body = resp
            .text()
            .await
            .map_err(|e| FetchError::NetworkError(e.to_string()))?;

        let markdown = parse_json_to_markdown(url, &json_body)?;

        let headers = HashMap::new();
        let latency = start.elapsed();

        Ok(FetchResponse {
            status: 200,
            headers,
            body: markdown.into_bytes(),
            content_type: "text/markdown".to_string(),
            latency,
        })
    }

    fn name(&self) -> Strategy {
        Strategy::PublicApi
    }
}

fn is_reddit_url(url: &str) -> bool {
    let host = extract_host(url);
    host == "reddit.com" || host == "www.reddit.com" || host == "old.reddit.com"
}

fn is_hn_url(url: &str) -> bool {
    let host = extract_host(url);
    host == "news.ycombinator.com" || host == "hackernews.com"
}

fn is_youtube_url(url: &str) -> bool {
    let host = extract_host(url);
    host == "youtube.com"
        || host == "www.youtube.com"
        || host == "youtu.be"
        || host == "m.youtube.com"
}

fn build_reddit_api_url(original: &str) -> Result<String, FetchError> {
    let path = extract_path(original);
    let path_without_query = path.split('?').next().unwrap_or(path);
    let trimmed = path_without_query.trim_end_matches('/');

    let mut api_url = format!("https://www.reddit.com{trimmed}");
    if !api_url.ends_with(".json") {
        api_url.push_str(".json");
    }
    if let Some(query) = extract_query(original) {
        api_url.push('?');
        api_url.push_str(query);
    }
    Ok(api_url)
}

fn build_hn_api_url(original: &str) -> Result<String, FetchError> {
    let query = extract_query(original).unwrap_or("");
    let id = query
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            let key = kv.next().unwrap_or("");
            let value = kv.next().unwrap_or("");
            if key == "id" && !value.is_empty() {
                Some(value.to_string())
            } else {
                None
            }
        })
        .next();

    let id = id.ok_or_else(|| FetchError::NetworkError("HN URL missing item id".to_string()))?;

    Ok(format!(
        "https://hacker-news.firebaseio.com/v0/item/{id}.json"
    ))
}

fn build_youtube_api_url(original: &str) -> Result<String, FetchError> {
    let encoded = percent_encode(original);
    Ok(format!("https://noembed.com/embed?url={encoded}"))
}

fn parse_json_to_markdown(original_url: &str, json: &str) -> Result<String, FetchError> {
    if is_reddit_url(original_url) {
        parse_reddit_json(json)
    } else if is_hn_url(original_url) {
        parse_hn_json(json)
    } else if is_youtube_url(original_url) {
        parse_youtube_json(json)
    } else {
        Err(FetchError::NetworkError(
            "no parser for this URL".to_string(),
        ))
    }
}

fn parse_reddit_json(json: &str) -> Result<String, FetchError> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| FetchError::NetworkError(format!("reddit JSON parse error: {e}")))?;

    let mut md = String::new();

    if let Some(arr) = val.as_array() {
        if let Some(post_data) = arr
            .first()
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("children"))
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .and_then(|c| c.get("data"))
        {
            let title = post_data
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("(untitled)");
            let subreddit = post_data
                .get("subreddit")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let author = post_data
                .get("author")
                .and_then(|v| v.as_str())
                .unwrap_or("[deleted]");
            let score = post_data.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
            let selftext = post_data
                .get("selftext")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            md.push_str(&format!("# {title}\n\n"));
            md.push_str(&format!("**r/{subreddit}** · u/{author} · ⬆ {score}\n\n"));
            if !selftext.is_empty() {
                md.push_str(selftext);
                md.push_str("\n\n");
            }
        }

        if let Some(comments_listing) = arr.get(1) {
            let top_comments = comments_listing
                .get("data")
                .and_then(|d| d.get("children"))
                .and_then(|c| c.as_array());

            if let Some(children) = top_comments {
                let mut count = 0;
                md.push_str("---\n\n## Top Comments\n\n");
                for child in children {
                    if count >= 5 {
                        break;
                    }
                    let cdata = match child.get("data") {
                        Some(d) => d,
                        None => continue,
                    };
                    let cauthor = cdata
                        .get("author")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[deleted]");
                    let cscore = cdata.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
                    let cbody = cdata.get("body").and_then(|v| v.as_str()).unwrap_or("");

                    md.push_str(&format!("**u/{cauthor}** (⬆ {cscore})\n\n"));
                    md.push_str(cbody);
                    md.push_str("\n\n---\n\n");
                    count += 1;
                }
            }
        }
    }

    if md.is_empty() {
        md.push_str("(no content returned from Reddit API)\n");
    }

    Ok(md)
}

fn parse_hn_json(json: &str) -> Result<String, FetchError> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| FetchError::NetworkError(format!("HN JSON parse error: {e}")))?;

    let mut md = String::new();

    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(untitled)");
    let score = val.get("score").and_then(|v| v.as_i64()).unwrap_or(0);
    let by = val
        .get("by")
        .and_then(|v| v.as_str())
        .unwrap_or("[unknown]");
    let kids_count = val.get("descendants").and_then(|v| v.as_i64()).unwrap_or(0);

    md.push_str(&format!("# {title}\n\n"));
    md.push_str(&format!(
        "**Hacker News** · by {by} · ⬆ {score} · 💬 {kids_count}\n\n"
    ));

    if let Some(text) = val.get("text").and_then(|v| v.as_str()) {
        md.push_str(text);
        md.push_str("\n\n");
    }

    if let Some(link) = val.get("url").and_then(|v| v.as_str()) {
        md.push_str(&format!("**Link:** [{link}]({link})\n"));
    }

    if md.is_empty() {
        md.push_str("(no content returned from HN API)\n");
    }

    Ok(md)
}

fn parse_youtube_json(json: &str) -> Result<String, FetchError> {
    let val: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| FetchError::NetworkError(format!("YouTube JSON parse error: {e}")))?;

    let mut md = String::new();

    let title = val
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(untitled)");
    let author = val
        .get("author_name")
        .and_then(|v| v.as_str())
        .unwrap_or("[unknown]");
    let thumbnail = val
        .get("thumbnail_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    md.push_str(&format!("# {title}\n\n"));
    md.push_str(&format!("**Channel:** {author}\n\n"));

    if !thumbnail.is_empty() {
        md.push_str(&format!("![thumbnail]({thumbnail})\n\n"));
    }

    if let Some(error) = val.get("error").and_then(|v| v.as_str()) {
        md.push_str(&format!("⚠ _{error}_\n"));
    }

    if md.is_empty() {
        md.push_str("(no content returned from noembed)\n");
    }

    Ok(md)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_reddit_url() {
        assert!(is_reddit_url("https://reddit.com/r/rust/comments/abc123"));
        assert!(is_reddit_url("https://www.reddit.com/r/rust"));
        assert!(is_reddit_url("https://old.reddit.com/r/rust"));
        assert!(!is_reddit_url("https://news.ycombinator.com"));
    }

    #[test]
    fn test_is_hn_url() {
        assert!(is_hn_url("https://news.ycombinator.com/item?id=12345"));
        assert!(is_hn_url("https://hackernews.com/item?id=12345"));
        assert!(!is_hn_url("https://reddit.com"));
    }

    #[test]
    fn test_is_youtube_url() {
        assert!(is_youtube_url("https://youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://www.youtube.com/watch?v=abc"));
        assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://m.youtube.com/watch?v=abc"));
        assert!(!is_youtube_url("https://reddit.com"));
    }

    #[test]
    fn test_reddit_api_url() {
        let url = build_reddit_api_url("https://reddit.com/r/rust/comments/abc123/a_post/")
            .expect("reddit url");
        assert_eq!(
            url,
            "https://www.reddit.com/r/rust/comments/abc123/a_post.json"
        );
    }

    #[test]
    fn test_reddit_api_url_with_query() {
        let url = build_reddit_api_url("https://reddit.com/r/rust?sort=top").expect("reddit url");
        assert_eq!(url, "https://www.reddit.com/r/rust.json?sort=top");
    }

    #[test]
    fn test_hn_api_url_query_param() {
        let url = build_hn_api_url("https://news.ycombinator.com/item?id=12345").expect("hn url");
        assert_eq!(url, "https://hacker-news.firebaseio.com/v0/item/12345.json");
    }

    #[test]
    fn test_youtube_api_url() {
        let url = build_youtube_api_url("https://youtube.com/watch?v=dQw4w9WgXcQ").expect("yt url");
        assert!(url.starts_with("https://noembed.com/embed?url="));
        assert!(url.contains("youtube.com"));
    }

    #[test]
    fn test_unsupported_url() {
        let result = is_reddit_url("https://example.com")
            || is_hn_url("https://example.com")
            || is_youtube_url("https://example.com");
        assert!(!result);
    }

    #[test]
    fn test_parse_reddit_json() {
        let mock = r#"[
            {
                "data": {
                    "children": [{
                        "data": {
                            "title": "Rust is great",
                            "subreddit": "rust",
                            "author": "rustacean",
                            "score": 42,
                            "selftext": "Rust is a systems programming language."
                        }
                    }]
                }
            },
            {
                "data": {
                    "children": [
                        {"data": {"author": "commenter1", "score": 10, "body": "I agree!"}},
                        {"data": {"author": "commenter2", "score": 5, "body": "Nice post."}}
                    ]
                }
            }
        ]"#;

        let md = parse_reddit_json(mock).expect("parse reddit");
        assert!(md.contains("# Rust is great"));
        assert!(md.contains("r/rust"));
        assert!(md.contains("u/rustacean"));
        assert!(md.contains("systems programming language"));
        assert!(md.contains("Top Comments"));
        assert!(md.contains("I agree!"));
    }

    #[test]
    fn test_parse_hn_json_link_post() {
        let mock = r#"{
            "title": "Show HN: A cool project",
            "by": "pg",
            "score": 256,
            "descendants": 42,
            "url": "https://example.com/project"
        }"#;

        let md = parse_hn_json(mock).expect("parse hn");
        assert!(md.contains("# Show HN: A cool project"));
        assert!(md.contains("by pg"));
        assert!(md.contains("⬆ 256"));
        assert!(md.contains("example.com/project"));
    }

    #[test]
    fn test_parse_hn_json_ask_post() {
        let mock = r#"{
            "title": "Ask HN: What do you think of Rust?",
            "by": "user1",
            "score": 100,
            "descendants": 80,
            "text": "<p>I am curious about Rust.</p>"
        }"#;

        let md = parse_hn_json(mock).expect("parse hn");
        assert!(md.contains("# Ask HN"));
        assert!(md.contains("I am curious about Rust"));
    }

    #[test]
    fn test_parse_youtube_json() {
        let mock = r#"{
            "title": "Amazing Video",
            "author_name": "Cool Channel",
            "thumbnail_url": "https://i.ytimg.com/vi/abc/hqdefault.jpg"
        }"#;

        let md = parse_youtube_json(mock).expect("parse youtube");
        assert!(md.contains("# Amazing Video"));
        assert!(md.contains("Cool Channel"));
        assert!(md.contains("thumbnail"));
    }

    #[test]
    fn test_backend_name() {
        let backend = PublicApiBackend::new().expect("backend creation");
        assert_eq!(backend.name(), Strategy::PublicApi);
    }

    #[test]
    fn test_backend_new() {
        assert!(PublicApiBackend::new().is_ok());
    }

    #[test]
    fn test_hn_url_missing_id() {
        assert!(build_hn_api_url("https://news.ycombinator.com/").is_err());
    }

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(
            percent_encode("https://youtube.com/watch?v=abc"),
            "https%3A%2F%2Fyoutube.com%2Fwatch%3Fv%3Dabc"
        );
        assert_eq!(percent_encode("abc-123_."), "abc-123_.");
    }
}
