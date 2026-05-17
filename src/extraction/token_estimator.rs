//! Token count estimation using a simple character-based heuristic.

pub struct TokenEstimator;

impl TokenEstimator {
    /// Estimate token count (≈4 chars per token for English).
    pub fn estimate(text: &str) -> u32 {
        (text.len() as u32) / 4
    }

    /// When truncation occurs, a Table of Contents extracted from all headings
    /// is prepended to the truncated content, giving the caller an overview of the full document.
    pub fn truncate_smart(text: &str, budget: u32) -> (String, bool) {
        let max_chars = (budget as usize) * 4;
        if text.len() <= max_chars {
            return (text.to_string(), false);
        }

        let headings: Vec<&str> = text.lines().filter(|l| l.trim().starts_with('#')).collect();

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

        if result.len() > max_chars * 2 && last_heading_break > 0 {
            result.truncate(last_heading_break);
        }

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

    pub fn truncate(content: &str, max_tokens: usize) -> String {
        let max_chars = max_tokens * 4;
        let marker = "\n\n[... truncated ...]";
        let marker_chars = marker.len();
        let usable_chars = max_chars.saturating_sub(marker_chars);

        if content.len() <= max_chars {
            return content.to_string();
        }

        let sections = Self::split_into_sections(content);

        if sections.len() <= 1 {
            if usable_chars >= content.len() {
                return content.to_string();
            }
            let cut = &content[..usable_chars.min(content.len())];
            return format!("{}{marker}", cut.trim_end());
        }

        let scored: Vec<(usize, &str, f32)> = sections
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut score = 0.0f32;
                if i == 0 {
                    score += 100.0;
                }
                if s.contains("```") {
                    score += 30.0;
                }
                if s.trim().starts_with('#') {
                    score += 10.0;
                }
                score -= (s.len() as f32).log2() * 2.0;
                (i, *s, score)
            })
            .collect();

        let mut sorted: Vec<(usize, &str, f32)> = scored.into_iter().collect();
        sorted.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        sorted.sort_by_key(|(i, _, _)| *i);

        let mut included: Vec<&str> = Vec::new();
        let mut total_chars: usize = 0;

        for &(_, section, _) in &sorted {
            let needed = if included.is_empty() {
                section.len()
            } else {
                section.len() + 2
            };
            if total_chars + needed <= usable_chars {
                included.push(section);
                total_chars += needed;
            }
        }

        included.sort_by_key(|s| content.find(*s).unwrap_or(0));

        let mut result = included.join("\n\n");
        if result.len() < content.len() {
            result.push_str(marker);
        }

        result
    }

    fn split_into_sections(content: &str) -> Vec<&str> {
        let has_h2 = content.contains("\n## ");
        if has_h2 {
            let mut sections: Vec<&str> = Vec::new();
            let mut last = 0;
            for (i, _) in content.match_indices("\n## ") {
                if last < i {
                    sections.push(&content[last..i]);
                }
                last = i + 1;
            }
            if last < content.len() {
                sections.push(&content[last..]);
            }
            if sections.is_empty() {
                vec![content]
            } else {
                sections
            }
        } else {
            content.split("\n\n").collect()
        }
    }

    /// Truncate text to fit within `budget` tokens, breaking at paragraph → sentence → word boundary.
    pub fn truncate_to_budget(text: &str, budget: u32) -> (String, bool) {
        let max_chars = (budget as usize) * 4;
        if text.len() <= max_chars {
            return (text.to_string(), false);
        }

        if let Some(pos) = text[..max_chars].rfind("\n\n") {
            return (text[..pos].to_string(), true);
        }

        if let Some(pos) = text[..max_chars].rfind(". ") {
            return (text[..pos + 1].to_string(), true);
        }

        if let Some(pos) = text[..max_chars].rfind(' ') {
            return (text[..pos].to_string(), true);
        }

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

    #[test]
    fn truncate_within_budget_returns_unchanged() {
        let text = "Short content that fits easily.";
        let result = TokenEstimator::truncate(text, 1000);
        assert_eq!(result, text);
    }

    #[test]
    fn truncate_section_aware_cuts_at_heading_boundary() {
        let text = "# Intro\n\nIntro paragraph.\n\n## Section One\n\nSome prose here.\n\n## Section Two\n\n".to_string()
            + &"more text ".repeat(200)
            + "\n\n## Section Three\n\nFinal section.";
        let budget = 100;
        let result = TokenEstimator::truncate(&text, budget);
        assert!(result.contains("# Intro"), "must keep intro");
        assert!(result.contains("## Section One"), "must keep section one");
        assert!(
            result.contains("[... truncated ...]"),
            "must have truncation marker: {result}"
        );
    }

    #[test]
    fn truncate_preserves_code_sections_over_prose() {
        let code_block = "```\nfn main() { println!(\"hello\"); }\n```";
        let prose = &"lorem ipsum ".repeat(200);
        let text = format!("# Intro\n\n{prose}\n\n## Code\n\n{code_block}");
        let budget = 80;
        let result = TokenEstimator::truncate(&text, budget);
        assert!(
            result.contains("```"),
            "code blocks should get priority bonus: {result}"
        );
        assert!(result.contains("[... truncated ...]"));
    }

    #[test]
    fn truncate_marker_present_when_cut() {
        let text = "a".repeat(1000);
        let result = TokenEstimator::truncate(&text, 10);
        assert!(result.contains("[... truncated ...]"));
        assert!(result.len() < text.len());
    }

    #[test]
    fn truncate_empty_content() {
        let result = TokenEstimator::truncate("", 100);
        assert_eq!(result, "");
    }

    #[test]
    fn truncate_zero_budget() {
        let text = "Some content here.";
        let result = TokenEstimator::truncate(text, 0);
        assert!(result.contains("[... truncated ...]") || result.is_empty());
    }
}
