//! Content extraction pipeline: HTML → readability → clean Markdown.
//!
//! Uses dom_smoothie to extract the main content from web pages,
//! stripping navigation, headers, footers, and other boilerplate.

pub mod boilerplate;
pub mod markdown;
pub mod scoring;
pub mod structural;
pub mod token_estimator;

/// Extracted content from a web page.
#[derive(Debug, Clone)]
pub struct ExtractedContent {
    /// Page title.
    pub title: String,
    /// Readability-processed HTML content.
    pub content: String,
    /// Markdown output from readability.
    pub text_content: String,
    /// Source URL.
    pub url: String,
    /// Length of the text content in bytes.
    pub length: usize,
    /// Content-Type from the HTTP response (e.g. "text/html").
    pub content_type: String,
}

/// Extract main content from an HTML page.
///
/// Returns `Ok(None)` if the HTML cannot be parsed or no meaningful content
/// is found (e.g., empty input, boilerplate-only pages).
///
/// # Errors
///
/// Returns an error only for unexpected failures (e.g., invalid URL format).
pub fn extract(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let ct_lower = content_type.to_lowercase();

    // JSON: pretty-print and return as-is.
    if ct_lower.contains("application/json") {
        return extract_json(html, url, content_type);
    }

    // Plain text: return as-is.
    if ct_lower.contains("text/plain") {
        return Ok(Some(ExtractedContent {
            title: extract_title_from_url(url),
            content: html.to_string(),
            text_content: html.to_string(),
            url: url.to_string(),
            length: html.len(),
            content_type: content_type.to_string(),
        }));
    }

    if ct_lower.contains("application/pdf")
        || ct_lower.contains("application/octet-stream")
        || ct_lower.contains("image/")
        || ct_lower.contains("video/")
        || ct_lower.contains("audio/")
    {
        return Ok(Some(ExtractedContent {
            title: String::new(),
            content: String::new(),
            text_content: format!(
                "[Unsupported content type: {content_type}. Binary content cannot be extracted.]"
            ),
            url: url.to_string(),
            length: 0,
            content_type: content_type.to_string(),
        }));
    }

    extract_html(html, url, content_type)
}

fn extract_html(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let cfg = dom_smoothie::Config {
        text_mode: dom_smoothie::TextMode::Markdown,
        ..Default::default()
    };

    let mut readability = match dom_smoothie::Readability::new(html, Some(url), Some(cfg)) {
        Ok(r) => r,
        Err(_) => return Ok(None),
    };

    let article = match readability.parse() {
        Ok(a) => a,
        Err(_) => return Ok(None),
    };

    if article.text_content.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(ExtractedContent {
        title: article.title,
        content: article.content.to_string(),
        text_content: markdown::clean_markdown(&article.text_content),
        url: url.to_string(),
        length: article.length,
        content_type: content_type.to_string(),
    }))
}

fn extract_json(
    raw: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let pretty = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| raw.to_string());

    Ok(Some(ExtractedContent {
        title: extract_title_from_url(url),
        content: pretty.clone(),
        text_content: pretty,
        url: url.to_string(),
        length: raw.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_title_from_url(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('?')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// Extract content and filter by relevance to the given focus terms.
///
/// When `focus` is non-empty, the extracted Markdown is split into sections,
/// scored by keyword overlap, and only relevant sections are kept.
/// Falls back to the full extraction when focus is empty or no sections score above zero.
pub fn scored_extract(
    html: &str,
    url: &str,
    content_type: &str,
    focus: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let extracted = extract(html, url, content_type)?;
    let Some(mut content) = extracted else {
        return Ok(None);
    };

    if focus.trim().is_empty() {
        return Ok(Some(content));
    }

    let sections = scoring::score_sections(&content.text_content, focus);
    let relevant: String = sections
        .iter()
        .take_while(|s| s.score > 0.0)
        .map(|s| {
            if s.heading.is_empty() {
                s.content.clone()
            } else {
                format!("{}\n{}", s.heading, s.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    if relevant.is_empty() {
        return Ok(Some(content));
    }

    content.text_content = relevant;
    content.length = content.text_content.len();
    Ok(Some(content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_blog_post() {
        let html = include_str!("../../tests/fixtures/blog_post.html");
        let result = extract(html, "https://example.com/blog/rust-ownership", "text/html")
            .expect("extraction should not error")
            .expect("should extract content");

        assert!(
            result.title.contains("Ownership"),
            "title should contain 'Ownership': got {:?}",
            result.title
        );
        assert!(
            result.text_content.contains("ownership model"),
            "article body should be preserved"
        );
        assert!(
            result.text_content.contains("calculate_length"),
            "code blocks should be preserved"
        );
        assert!(
            !result.text_content.contains("[Home]"),
            "nav links should be stripped"
        );
        assert!(
            !result.text_content.contains("Privacy Policy"),
            "footer should be stripped"
        );
    }

    #[test]
    fn test_extract_minimal_article() {
        let html = include_str!("../../tests/fixtures/minimal_article.html");
        let result = extract(html, "https://example.com/tip/cargo-watch", "text/html")
            .expect("extraction should not error")
            .expect("should extract content");

        assert!(
            result.text_content.contains("cargo watch"),
            "should contain 'cargo watch': got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("cargo install cargo-watch"),
            "code block should be preserved"
        );
        assert!(
            result.title.contains("cargo watch"),
            "title should reference cargo watch"
        );
    }

    #[test]
    fn test_extract_returns_none_for_empty() {
        let result =
            extract("", "https://example.com/empty", "text/html").expect("should not error");
        assert!(result.is_none(), "empty HTML should return None");

        let result = extract(
            "<html><body></body></html>",
            "https://example.com/blank",
            "text/html",
        )
        .expect("should not error");
        assert!(result.is_none(), "blank HTML should return None");
    }

    #[test]
    fn test_extract_preserves_code_blocks() {
        let html = include_str!("../../tests/fixtures/docs_page.html");
        let result = extract(html, "https://example.com/docs/api", "text/html")
            .expect("extraction should not error")
            .expect("should extract content");

        assert!(
            result.text_content.contains("parse") || result.text_content.contains("mylib"),
            "code content should be preserved"
        );
        assert!(
            result.text_content.contains("cargo") || result.text_content.contains("Installation"),
            "installation section should be present"
        );
    }

    #[test]
    fn test_extract_json_content_type() {
        let json = r#"{"name":"kaelo","version":"0.1.0"}"#;
        let result = extract(json, "https://api.example.com/v1/info", "application/json")
            .expect("should not error")
            .expect("should extract content");

        assert!(result.text_content.contains("\"name\""));
        assert!(result.text_content.contains("\"kaelo\""));
        assert_eq!(result.content_type, "application/json");
    }

    #[test]
    fn test_extract_plain_text_content_type() {
        let text = "Hello, this is plain text.\nLine two.";
        let result = extract(text, "https://example.com/readme.txt", "text/plain")
            .expect("should not error")
            .expect("should extract content");

        assert_eq!(result.text_content, text);
        assert_eq!(result.content_type, "text/plain");
    }

    #[test]
    fn test_extract_pdf_returns_unsupported() {
        let result = extract(
            "%PDF-1.4 fake",
            "https://example.com/doc.pdf",
            "application/pdf",
        )
        .expect("should not error")
        .expect("should return content");

        assert!(result.text_content.contains("Unsupported content type"));
        assert_eq!(result.length, 0);
    }

    #[test]
    fn test_extract_html_still_works() {
        let html = "<html><body><article><h1>Test</h1><p>Hello</p></article></body></html>";
        let result = extract(html, "https://example.com/page", "text/html; charset=utf-8")
            .expect("should not error")
            .expect("should extract content");

        assert!(result.text_content.contains("Hello"));
    }
}
