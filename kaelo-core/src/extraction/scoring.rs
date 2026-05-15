//! Section-level relevance scoring for the extraction pipeline.
//!
//! When a `focus` parameter is provided, sections are scored by keyword overlap
//! and returned sorted by relevance. Only sections with a positive score are
//! included in the final output.

/// A Markdown section split by heading boundaries.
#[derive(Debug, Clone)]
pub struct Section {
    /// Heading text (without the `#` prefix).
    pub heading: String,
    /// Body content between this heading and the next.
    pub content: String,
    /// Heading level (1–6).
    pub level: u8,
    /// Relevance score computed against focus terms.
    pub score: f64,
}

/// Score sections by keyword overlap with focus terms.
///
/// Returns sections sorted by relevance score in descending order.
/// Sections with zero relevance are still included so callers can decide
/// the cutoff threshold.
pub fn score_sections(markdown: &str, focus: &str) -> Vec<Section> {
    let keywords = tokenize(focus);
    let sections = parse_sections(markdown);

    let mut scored: Vec<Section> = sections
        .into_iter()
        .map(|mut s| {
            s.score = compute_relevance(&s, &keywords);
            s
        })
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

/// Tokenize a focus string into lowercase keywords.
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Parse Markdown into sections by splitting on heading lines (`#` through `######`).
pub(crate) fn parse_sections(markdown: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_heading = String::new();
    let mut current_content = String::new();
    let mut current_level: u8 = 0;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let level = trimmed.chars().take_while(|c| *c == '#').count() as u8;
            if (1..=6).contains(&level) {
                let heading_text = trimmed[level as usize..].trim().to_string();

                if !current_heading.is_empty() || !current_content.is_empty() {
                    sections.push(Section {
                        heading: std::mem::take(&mut current_heading),
                        content: std::mem::take(&mut current_content),
                        level: current_level,
                        score: 0.0,
                    });
                }
                current_heading = heading_text;
                current_level = level;
                continue;
            }
        }

        if !current_content.is_empty() {
            current_content.push('\n');
        }
        current_content.push_str(line);
    }

    // Flush the last section.
    if !current_heading.is_empty() || !current_content.is_empty() {
        sections.push(Section {
            heading: current_heading,
            content: current_content,
            level: current_level,
            score: 0.0,
        });
    }

    sections
}

/// Compute relevance score for a section given focus keywords.
///
/// Scoring components:
/// - **Keyword ratio**: fraction of focus keywords present in the section.
/// - **Heading bonus**: heading matches are weighted 2× relative to body matches.
/// - **Length factor**: small logarithmic bonus for longer content (log2(len)/10).
fn compute_relevance(section: &Section, keywords: &[String]) -> f64 {
    if keywords.is_empty() {
        return 0.0;
    }

    let heading_lower = section.heading.to_lowercase();
    let content_lower = section.content.to_lowercase();
    let combined = format!("{} {}", heading_lower, content_lower);

    let total = keywords.len() as f64;

    let keyword_ratio = keywords
        .iter()
        .filter(|kw| combined.contains(kw.as_str()))
        .count() as f64
        / total;

    let length_factor = (section.content.len() as f64).log2().max(0.0) / 10.0;

    let heading_matches = keywords
        .iter()
        .filter(|kw| heading_lower.contains(kw.as_str()))
        .count() as f64;
    let heading_bonus = heading_matches / total;

    keyword_ratio + heading_bonus * 0.5 + length_factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize() {
        let tokens = tokenize("Rust async programming");
        assert_eq!(tokens, vec!["rust", "async", "programming"]);
    }

    #[test]
    fn test_tokenize_strips_punctuation() {
        let tokens = tokenize("Rust, async! programming?");
        assert_eq!(tokens, vec!["rust", "async", "programming"]);
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_parse_sections() {
        let md = "\
# Heading 1

Some content here.

## Heading 2

More content.

# Heading 3

Final content.";
        let sections = parse_sections(md);
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].heading, "Heading 1");
        assert_eq!(sections[0].level, 1);
        assert_eq!(sections[1].heading, "Heading 2");
        assert_eq!(sections[1].level, 2);
        assert_eq!(sections[2].heading, "Heading 3");
        assert_eq!(sections[2].level, 1);
    }

    #[test]
    fn test_parse_sections_no_headings() {
        let md = "Just some text\nwithout any headings.";
        let sections = parse_sections(md);
        assert_eq!(sections.len(), 1);
        assert!(sections[0].heading.is_empty());
        assert_eq!(sections[0].level, 0);
        assert!(sections[0].content.contains("Just some text"));
    }

    #[test]
    fn test_score_relevance_high_match() {
        let section = Section {
            heading: "Rust Programming".to_string(),
            content: "Learn async programming in Rust with examples.".to_string(),
            level: 1,
            score: 0.0,
        };
        let keywords = tokenize("Rust async programming");
        let score = compute_relevance(&section, &keywords);
        assert!(score >= 1.0, "expected high score, got {}", score);
    }

    #[test]
    fn test_score_relevance_no_match() {
        let section = Section {
            heading: "Cooking Recipes".to_string(),
            content: "How to bake a cake.".to_string(),
            level: 1,
            score: 0.0,
        };
        let keywords = tokenize("Rust async programming");
        let score = compute_relevance(&section, &keywords);
        assert!(
            score < 0.5,
            "expected low score for no keyword match, got {}",
            score
        );
    }

    #[test]
    fn test_score_sections_sorted() {
        let md = "\
# Rust Async

Rust async programming with tokio.

# Cooking

How to bake bread.

# Programming Guide

General programming concepts and async patterns.";
        let result = score_sections(md, "Rust async programming");

        assert!(!result.is_empty());
        assert!(
            result[0].heading.contains("Rust"),
            "most relevant section should be 'Rust Async', got {:?}",
            result[0].heading
        );
        for window in result.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "sections should be sorted by score descending: {} >= {}",
                window[0].score,
                window[1].score
            );
        }
    }

    #[test]
    fn test_score_sections_empty_focus() {
        let md = "# Some Heading\n\nContent here.";
        let result = score_sections(md, "");
        for s in &result {
            assert!(
                s.score == 0.0,
                "empty focus should yield 0.0 score, got {}",
                s.score
            );
        }
    }
}
