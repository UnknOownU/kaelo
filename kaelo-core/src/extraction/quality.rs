//! Content quality validation for cache eligibility.
//! Prevents caching empty/useless content like Blazor server shells and raw HTML artifacts.

const MIN_CHARS: usize = 200;
const MIN_LINES: usize = 5;

/// Patterns that indicate raw HTML leaked into markdown content.
/// Case-insensitive — all checked against lowercase content.
const HTML_ARTIFACT_PATTERNS: &[&str] = &[
    "<script",
    "<link",
    "<style",
    "<!doctype",
    "<html",
    "<head",
    "<body",
    "<!--blazor:",
    "blazor.server.js",
    "__next_data__",
    "__nuxt__",
];

/// Detects raw HTML artifacts that leaked into extracted content.
/// Uses fast string contains checks (no parsing).
/// Returns `true` if ANY structural HTML pattern is found.
fn is_html_artifact(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    HTML_ARTIFACT_PATTERNS
        .iter()
        .any(|&pattern| lower.contains(pattern))
}

/// Assessment of content quality.
#[derive(Debug, Clone)]
pub struct ContentQuality {
    /// Whether the content is valuable enough to cache.
    pub is_valuable: bool,
    /// Total character count.
    pub char_count: usize,
    /// Count of non-empty lines.
    pub line_count: usize,
    /// Reason for rejection, if not valuable.
    pub reason: Option<&'static str>,
}

impl ContentQuality {
    pub fn check(content: &str) -> Self {
        let char_count = content.len();
        let line_count = content.lines().filter(|l| !l.trim().is_empty()).count();

        // Fail fast on obvious HTML artifacts
        if is_html_artifact(content) {
            return Self {
                is_valuable: false,
                char_count,
                line_count,
                reason: Some("content contains raw HTML artifacts"),
            };
        }

        if char_count < MIN_CHARS {
            return Self {
                is_valuable: false,
                char_count,
                line_count,
                reason: Some("content too short"),
            };
        }

        if line_count < MIN_LINES {
            return Self {
                is_valuable: false,
                char_count,
                line_count,
                reason: Some("too few non-empty lines"),
            };
        }

        Self {
            is_valuable: true,
            char_count,
            line_count,
            reason: None,
        }
    }
}

/// Quick check: is this content valuable enough to cache?
pub fn is_content_valuable(content: &str) -> bool {
    ContentQuality::check(content).is_valuable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_not_valuable() {
        assert!(!is_content_valuable(""));
    }

    #[test]
    fn whitespace_only_not_valuable() {
        assert!(!is_content_valuable("   \n  \n  "));
    }

    #[test]
    fn too_short_not_valuable() {
        // 40 chars, 1 line — both too short and too few lines
        assert!(!is_content_valuable("This is a very short piece of text."));
    }

    #[test]
    fn html_shell_not_valuable() {
        // Realistic Blazor shell similar to sbox.game — ~30 lines, 1500+ chars
        // Should be rejected by is_html_artifact even though it exceeds char/line thresholds
        let shell = concat!(
            "<!DOCTYPE html>\n",
            "<html lang=\"en\">\n",
            "<head>\n",
            "    <meta charset=\"utf-8\" />\n",
            "    <meta name=\"theme-color\" content=\"#192337\">\n",
            "    <base href=\"/\" />\n",
            "    <link rel=\"icon\" href=\"/favicon.ico\" type=\"image/x-icon\">\n",
            "    <meta name=\"twitter:site\" content=\"@s8box\">\n",
            "    <link rel=\"alternate\" type=\"application/rss+xml\" title=\"s&box News Feed\" href=\"/rss/news\" />\n",
            "    <title>s&box</title>\n",
            "    <link rel=\"stylesheet\" href=\"/Works.styles.css\"/>\n",
            "    <link rel=\"stylesheet\" href=\"/styles.css\"/>\n",
            "    <!--Blazor:{\"type\":\"server\",\"key\":{...},\"sequence\":0,\"descriptor\":\"CfDJ8...\"}-->\n",
            "</head>\n",
            "<body>\n",
            "    <div id=\"components-reconnect-modal\"></div>\n",
            "    <!--Blazor:{\"type\":\"server\",\"key\":{...},\"sequence\":1,\"descriptor\":\"CfDJ8...\"}-->\n",
            "    <script>window.RequestToken = \"abc123\";</script>\n",
            "    <script src=\"_framework/blazor.server.js\" autostart=\"false\"></script>\n",
            "    <script src=\"/sbox.js\"></script>\n",
            "</body>\n",
            "</html>",
        );
        assert!(!is_content_valuable(shell));
        let q = ContentQuality::check(shell);
        assert_eq!(q.reason, Some("content contains raw HTML artifacts"));
    }

    #[test]
    fn raw_html_not_valuable() {
        // Content with <script> tags should be rejected
        let content = "# Welcome to my site\n\nSome intro text here.\n\n\
            <script src=\"analytics.js\"></script>\n\n\
            More content follows here.\n\n\
            And some final text for good measure.";
        assert!(!is_content_valuable(content));
        let q = ContentQuality::check(content);
        assert_eq!(q.reason, Some("content contains raw HTML artifacts"));
    }

    #[test]
    fn blazor_descriptor_not_valuable() {
        // Content with <!--Blazor: descriptor should be rejected
        let content = "Some text that might look like content\n\n\
            More lines of text here\n\n\
            <!--Blazor:{\"type\":\"server\",\"key\":1,\"descriptor\":\"abc\"}-->\n\n\
            Even more text follows\n\n\
            And final text too";
        assert!(!is_content_valuable(content));
        let q = ContentQuality::check(content);
        assert_eq!(q.reason, Some("content contains raw HTML artifacts"));
    }

    #[test]
    fn real_markdown_is_valuable() {
        // Actual markdown with 200+ chars and 5+ lines → valuable
        let content = "# Introduction to Rust Programming\n\n\
            Rust is a systems programming language focused on safety, speed, and concurrency. \
            It provides memory safety without garbage collection by using ownership and borrowing.\n\n\
            ## Key Features\n\n\
            - Zero-cost abstractions\n\
            - Memory safety without GC\n\
            - Concurrency without data races\n\
            - Rich type system with pattern matching";
        assert!(is_content_valuable(content));
    }

    #[test]
    fn json_not_affected() {
        // Short JSON string under 200 chars — not valuable due to length,
        // but NOT rejected as HTML artifact. JSON is handled before quality check
        // in extraction/mod.rs, so this confirms the pattern doesn't misfire.
        let content = "{\"name\":\"test\",\"value\":42}";
        assert!(!is_content_valuable(content));
        let q = ContentQuality::check(content);
        assert_eq!(q.reason, Some("content too short"));
    }
}
