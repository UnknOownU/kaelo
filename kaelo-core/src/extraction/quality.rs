/// Content quality validation for cache eligibility.
/// Prevents caching empty/useless content like Blazor server shells.

/// Minimum character count for content to be considered valuable.
const MIN_CHARS: usize = 50;
/// Minimum non-empty line count for content to be considered valuable.
const MIN_LINES: usize = 3;

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
        // Typical Blazor shell — short, few lines
        assert!(!is_content_valuable(
            "<div id=\"app\"></div>\n<script src=\"blazor.server.js\"></script>"
        ));
    }

    #[test]
    fn valid_content_is_valuable() {
        let content = "This is a paragraph with enough content to be considered valuable.\n\n\
            It has multiple lines of text that provide meaningful information.\n\n\
            The third line ensures we meet the minimum line count requirement for caching.";
        assert!(is_content_valuable(content));
    }

    #[test]
    fn content_quality_struct_fields() {
        let content = "short";
        let q = ContentQuality::check(content);
        assert!(!q.is_valuable);
        assert_eq!(q.char_count, 5);
        assert_eq!(q.line_count, 1);
        assert!(q.reason.is_some());
    }
}
