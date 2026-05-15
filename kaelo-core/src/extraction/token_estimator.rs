//! Token count estimation using a simple character-based heuristic.
//!
//! For production tokenization, use tiktoken or a proper tokenizer.
//! This module provides a fast, dependency-free estimate.

pub struct TokenEstimator;

impl TokenEstimator {
    /// Estimate token count. Heuristic: text.len() / 4.
    ///
    /// Roughly 4 characters per token for English text.
    pub fn estimate(text: &str) -> u32 {
        (text.len() as u32) / 4
    }

    /// Truncate Markdown content while preserving heading structure.
    ///
    /// Returns a tuple of (result, was_truncated). When truncation occurs,
    /// a Table of Contents extracted from all headings is prepended to the
    /// truncated content, giving the caller an overview of the full document.
    pub fn truncate_smart(text: &str, budget: u32) -> (String, bool) {
        let max_chars = (budget as usize) * 4;
        if text.len() <= max_chars {
            return (text.to_string(), false);
        }

        // Extract all headings for TOC
        let headings: Vec<&str> = text.lines().filter(|l| l.trim().starts_with('#')).collect();

        // Walk lines, stopping at the first heading that would exceed the budget
        let mut result = String::new();
        let mut last_heading_break = 0usize;

        for line in text.lines() {
            let new_len = result.len() + line.len() + 1;
            if new_len > max_chars && line.trim().starts_with('#') && !result.is_empty() {
                break;
            }
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
            if line.trim().starts_with('#') {
                last_heading_break = result.len();
            }
        }

        // Safety net: if result is still wildly over budget, cut at last heading
        if result.len() > max_chars * 2 && last_heading_break > 0 {
            result.truncate(last_heading_break);
        }

        // Prepend TOC when we have headings
        if !headings.is_empty() {
            let toc = headings
                .iter()
                .map(|h| format!("- {}", h.trim().trim_start_matches('#').trim()))
                .collect::<Vec<_>>()
                .join("\n");
            result = format!("## Table of Contents\n{}\n\n{}", toc, result);
        }

        (result, true)
    }

    /// Truncate text to fit within `budget` tokens, breaking at paragraph → sentence → word boundary.
    pub fn truncate_to_budget(text: &str, budget: u32) -> (String, bool) {
        let max_chars = (budget as usize) * 4;
        if text.len() <= max_chars {
            return (text.to_string(), false);
        }

        // Try paragraph boundary
        if let Some(pos) = text[..max_chars].rfind("\n\n") {
            return (text[..pos].to_string(), true);
        }

        // Try sentence boundary
        if let Some(pos) = text[..max_chars].rfind(". ") {
            return (text[..pos + 1].to_string(), true);
        }

        // Try word boundary
        if let Some(pos) = text[..max_chars].rfind(' ') {
            return (text[..pos].to_string(), true);
        }

        // Hard cut
        (text[..max_chars].to_string(), true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_short() {
        let tokens = TokenEstimator::estimate("Hello world");
        assert!(tokens == 2 || tokens == 3, "expected ~2-3, got {tokens}");
    }

    #[test]
    fn test_estimate_longer() {
        let text = "a".repeat(1000);
        let tokens = TokenEstimator::estimate(&text);
        assert_eq!(tokens, 250);
    }

    #[test]
    fn test_truncate_no_truncation_needed() {
        let text = "short text";
        let (result, was_truncated) = TokenEstimator::truncate_to_budget(text, 1000);
        assert_eq!(result, text);
        assert!(!was_truncated);
    }

    #[test]
    fn test_truncate_at_paragraph() {
        let text = "First paragraph here.\n\nSecond paragraph that should be cut off because budget is tight.";
        // Budget of 10 tokens = 40 chars, enough for first paragraph but not second
        let (result, was_truncated) = TokenEstimator::truncate_to_budget(text, 10);
        assert!(was_truncated);
        assert!(result.contains("First paragraph"));
        assert!(!result.contains("Second paragraph"));
        assert!(result.ends_with("\n\n") || !result.contains("Second"));
    }

    #[test]
    fn test_truncate_at_sentence() {
        // No paragraph breaks, but sentence breaks exist
        let text = "This is the first sentence. This is the second sentence. This is the third.";
        let budget = 15; // ~60 chars
        let (result, was_truncated) = TokenEstimator::truncate_to_budget(text, budget);
        assert!(was_truncated);
        assert!(result.contains("first sentence"));
    }

    #[test]
    fn test_truncate_at_word() {
        // No paragraph or sentence breaks, only spaces
        let text = "one two three four five six seven eight nine ten eleven twelve";
        let budget = 5; // ~20 chars
        let (result, was_truncated) = TokenEstimator::truncate_to_budget(text, budget);
        assert!(was_truncated);
        assert!(result.contains("one"));
    }

    #[test]
    fn test_truncate_returns_flag() {
        let text = "a".repeat(100);
        let (_, was_truncated) = TokenEstimator::truncate_to_budget(&text, 5);
        assert!(
            was_truncated,
            "should report truncation when text exceeds budget"
        );
    }

    #[test]
    fn test_smart_no_truncation_needed() {
        let text = "# Hello\n\nSome short text.";
        let (result, truncated) = TokenEstimator::truncate_smart(text, 1000);
        assert_eq!(result, text);
        assert!(!truncated);
    }

    #[test]
    fn test_smart_truncation_adds_toc() {
        let text = "# Intro\n\nIntro content here.\n\n# Chapter 1\n\n".to_string()
            + &"word ".repeat(500)
            + "\n\n# Chapter 2\n\nMore content here.";
        let (result, truncated) = TokenEstimator::truncate_smart(&text, 50);
        assert!(truncated);
        assert!(
            result.contains("## Table of Contents"),
            "should include TOC when truncated: {result}"
        );
        assert!(result.contains("- Intro"));
        assert!(result.contains("- Chapter 1"));
        assert!(result.contains("- Chapter 2"));
    }

    #[test]
    fn test_smart_truncation_no_headings() {
        let text = "word ".repeat(500);
        let (result, truncated) = TokenEstimator::truncate_smart(&text, 50);
        assert!(truncated);
        assert!(
            !result.contains("Table of Contents"),
            "no TOC when no headings"
        );
    }

    #[test]
    fn test_smart_breaks_at_heading_boundary() {
        let mut text = "# First\n\n".to_string();
        for i in 0..100 {
            text.push_str(&format!("Line {} of content.\n", i));
        }
        text.push_str("\n# Second\n\nSecond section content.");
        let (result, truncated) = TokenEstimator::truncate_smart(&text, 30);
        assert!(truncated);
        assert!(result.contains("# First"), "should include first heading");
        assert!(
            !result.contains("# Second"),
            "should stop before second heading"
        );
    }
}
