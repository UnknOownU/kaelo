/// Section-level structural fingerprints per domain.
///
/// Tracks heading patterns (common levels, repeated section headings) observed
/// across pages from the same domain, then uses this knowledge to improve
/// section relevance scoring.
use std::collections::HashMap;

use super::scoring::Section;

/// Per-domain structural profile.
#[derive(Debug, Clone, Default)]
pub struct DomainProfile {
    /// heading_text → frequency count across observed pages.
    pub heading_freq: HashMap<String, u32>,
    /// heading_level → frequency count.
    pub level_freq: HashMap<u8, u32>,
    /// Total pages observed for this domain.
    pub pages_observed: u32,
}

/// In-memory tracker that accumulates per-domain heading statistics.
pub struct StructuralTracker {
    profiles: HashMap<String, DomainProfile>,
}

/// Minimum frequency ratio (relative to pages observed) for a heading to be
/// considered a "structural" (repeated) section of the domain.
const STRUCTURAL_RATIO: f64 = 0.5;

impl Default for StructuralTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl StructuralTracker {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Record the heading structure observed on a page for `domain`.
    pub fn record_page(&mut self, domain: &str, sections: &[Section]) {
        let profile = self.profiles.entry(domain.to_string()).or_default();
        profile.pages_observed += 1;

        for section in sections {
            if section.heading.is_empty() {
                continue;
            }
            *profile
                .heading_freq
                .entry(section.heading.to_lowercase())
                .or_insert(0) += 1;
            *profile.level_freq.entry(section.level).or_insert(0) += 1;
        }
    }

    /// Boost the scores of sections that are *not* structural boilerplate
    /// headings, and penalise structural (repeated) headings.
    ///
    /// Returns a modified list of sections with adjusted scores.
    pub fn adjust_scores(&self, domain: &str, sections: &mut [Section]) {
        let Some(profile) = self.profiles.get(domain) else {
            return;
        };

        if profile.pages_observed == 0 {
            return;
        }

        for section in sections.iter_mut() {
            let key = section.heading.to_lowercase();
            if let Some(freq) = profile.heading_freq.get(&key) {
                let ratio = *freq as f64 / profile.pages_observed as f64;
                if ratio >= STRUCTURAL_RATIO {
                    // Penalise structural / repeated headings.
                    section.score *= 1.0 - ratio;
                }
            }
        }
    }

    /// Return the most common heading level for a domain, if known.
    pub fn dominant_level(&self, domain: &str) -> Option<u8> {
        let profile = self.profiles.get(domain)?;
        profile
            .level_freq
            .iter()
            .max_by_key(|(_, count)| *count)
            .map(|(level, _)| *level)
    }

    /// Return heading texts that are classified as structural for `domain`.
    pub fn structural_headings(&self, domain: &str) -> Vec<String> {
        let Some(profile) = self.profiles.get(domain) else {
            return vec![];
        };

        if profile.pages_observed == 0 {
            return vec![];
        }

        profile
            .heading_freq
            .iter()
            .filter(|(_, freq)| **freq as f64 / profile.pages_observed as f64 >= STRUCTURAL_RATIO)
            .map(|(heading, _)| heading.clone())
            .collect()
    }

    /// Clear tracking data for a domain.
    pub fn clear_domain(&mut self, domain: &str) {
        self.profiles.remove(domain);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::scoring::Section;

    fn make_section(heading: &str, level: u8, score: f64) -> Section {
        Section {
            heading: heading.to_string(),
            content: format!("Content of {heading}"),
            level,
            score,
        }
    }

    #[test]
    fn test_structural_heading_detection() {
        let mut tracker = StructuralTracker::new();
        let domain = "example.com";

        for _ in 0..4 {
            let sections = vec![
                make_section("Navigation", 2, 1.0),
                make_section("Related Articles", 3, 1.0),
                make_section("Main Content", 2, 1.0),
            ];
            tracker.record_page(domain, &sections);
        }

        let structural = tracker.structural_headings(domain);
        assert!(structural.contains(&"navigation".to_string()));
        assert!(structural.contains(&"related articles".to_string()));
    }

    #[test]
    fn test_adjust_scores_penalises_structural() {
        let mut tracker = StructuralTracker::new();
        let domain = "example.com";

        for i in 0..4 {
            let sections = vec![
                make_section("Footer", 2, 1.0),
                make_section(&format!("Article {i}"), 2, 1.0),
            ];
            tracker.record_page(domain, &sections);
        }

        let mut sections = vec![
            make_section("Footer", 2, 10.0),
            make_section("Article 0", 2, 10.0),
        ];
        tracker.adjust_scores(domain, &mut sections);

        let footer = sections.iter().find(|s| s.heading == "Footer").unwrap();
        let article = sections.iter().find(|s| s.heading == "Article 0").unwrap();
        assert!(footer.score < article.score);
    }

    #[test]
    fn test_dominant_level() {
        let mut tracker = StructuralTracker::new();
        let domain = "example.com";

        let sections = vec![
            make_section("A", 2, 1.0),
            make_section("B", 2, 1.0),
            make_section("C", 3, 1.0),
        ];
        tracker.record_page(domain, &sections);

        assert_eq!(tracker.dominant_level(domain), Some(2));
    }

    #[test]
    fn test_unknown_domain_no_panic() {
        let tracker = StructuralTracker::new();
        assert_eq!(tracker.dominant_level("unknown.com"), None);
        assert!(tracker.structural_headings("unknown.com").is_empty());

        let mut sections = vec![make_section("Test", 1, 5.0)];
        tracker.adjust_scores("unknown.com", &mut sections);
        assert_eq!(sections[0].score, 5.0);
    }
}
