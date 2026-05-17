/// Domain boilerplate fingerprinting.
///
/// Tracks per-domain boilerplate patterns (nav text, sidebar text, footer text).
/// When extracting content, detects and removes previously-seen boilerplate.
///
/// Simple approach: hash text blocks; if a block appears on 3+ pages from the
/// same domain, it is marked as boilerplate and stripped from future extractions.
use std::collections::HashMap;

use sha2::{Digest, Sha256};

const BOILERPLATE_THRESHOLD: u32 = 3;

/// Larger blocks are probably unique content, not boilerplate.
const MAX_BLOCK_CHARS: usize = 2000;

/// In-memory tracker that accumulates per-domain block frequencies and
/// identifies boilerplate blocks once seen on enough distinct pages.
pub struct BoilerplateTracker {
    /// domain → (block_hash → page_count)
    frequencies: HashMap<String, HashMap<String, u32>>,
    /// Pre-computed boilerplate set per domain for fast lookups.
    /// Lazily rebuilt when frequencies change.
    boilerplate: HashMap<String, HashMap<String, ()>>,
    dirty: HashMap<String, bool>,
}

impl Default for BoilerplateTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl BoilerplateTracker {
    pub fn new() -> Self {
        Self {
            frequencies: HashMap::new(),
            boilerplate: HashMap::new(),
            dirty: HashMap::new(),
        }
    }

    /// Record text blocks extracted from a single page.
    ///
    /// `domain` is the site domain (e.g. `"example.com"`).
    /// `blocks` are the individual text chunks (paragraphs, list items, etc.).
    pub fn record_page(&mut self, domain: &str, blocks: &[&str]) {
        let domain_freqs = self.frequencies.entry(domain.to_string()).or_default();

        for block in blocks {
            if block.len() > MAX_BLOCK_CHARS || block.trim().is_empty() {
                continue;
            }
            let hash = sha256_hex(block.as_bytes());
            let counter = domain_freqs.entry(hash).or_insert(0);
            *counter += 1;
        }

        self.dirty.insert(domain.to_string(), true);
    }

    pub fn is_boilerplate(&mut self, domain: &str, block: &str) -> bool {
        if block.len() > MAX_BLOCK_CHARS || block.trim().is_empty() {
            return false;
        }

        if self.dirty.get(domain).copied().unwrap_or(false) {
            self.rebuild_boilerplate(domain);
        }

        let hash = sha256_hex(block.as_bytes());
        self.boilerplate
            .get(domain)
            .map(|set| set.contains_key(&hash))
            .unwrap_or(false)
    }

    /// Splits the text into lines, removes any line whose trimmed content
    /// matches a known boilerplate fingerprint, and reassembles.
    pub fn strip_boilerplate(&mut self, domain: &str, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        let kept: Vec<&str> = lines
            .into_iter()
            .filter(|line| !self.is_boilerplate(domain, line.trim()))
            .collect();
        kept.join("\n")
    }

    pub fn boilerplate_count(&mut self, domain: &str) -> usize {
        if self.dirty.get(domain).copied().unwrap_or(false) {
            self.rebuild_boilerplate(domain);
        }
        self.boilerplate.get(domain).map(|s| s.len()).unwrap_or(0)
    }

    pub fn clear_domain(&mut self, domain: &str) {
        self.frequencies.remove(domain);
        self.boilerplate.remove(domain);
        self.dirty.remove(domain);
    }

    fn rebuild_boilerplate(&mut self, domain: &str) {
        let set = self
            .frequencies
            .get(domain)
            .map(|freqs| {
                freqs
                    .iter()
                    .filter(|(_, count)| **count >= BOILERPLATE_THRESHOLD)
                    .map(|(hash, _)| (hash.clone(), ()))
                    .collect::<HashMap<String, ()>>()
            })
            .unwrap_or_default();

        self.boilerplate.insert(domain.to_string(), set);
        self.dirty.insert(domain.to_string(), false);
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boilerplate_detection() {
        let mut tracker = BoilerplateTracker::new();
        let domain = "example.com";

        let nav = "Home About Contact";
        let footer = "Copyright 2024 Example Inc";

        // Simulate 3 pages with the same nav and footer, but different unique content.
        tracker.record_page(domain, &[nav, footer, "Unique content page 1"]);
        tracker.record_page(domain, &[nav, footer, "Unique content page 2"]);
        tracker.record_page(domain, &[nav, footer, "Unique content page 3"]);

        assert!(tracker.is_boilerplate(domain, nav));
        assert!(tracker.is_boilerplate(domain, footer));
        assert!(!tracker.is_boilerplate(domain, "Unique content page 1"));
    }

    #[test]
    fn test_strip_boilerplate() {
        let mut tracker = BoilerplateTracker::new();
        let domain = "example.com";

        let nav = "Home About Contact";
        let footer = "Copyright 2024 Example Inc";

        for _ in 0..3 {
            tracker.record_page(domain, &[nav, footer]);
        }

        let text = format!("{}\nReal article content\n{}", nav, footer);
        let stripped = tracker.strip_boilerplate(domain, &text);

        assert!(!stripped.contains(nav));
        assert!(!stripped.contains(footer));
        assert!(stripped.contains("Real article content"));
    }

    #[test]
    fn test_below_threshold() {
        let mut tracker = BoilerplateTracker::new();
        let domain = "example.com";

        // Only 2 pages — below threshold of 3.
        for _ in 0..2 {
            tracker.record_page(domain, &["Repeated block"]);
        }

        assert!(!tracker.is_boilerplate(domain, "Repeated block"));
    }

    #[test]
    fn test_different_domains_isolated() {
        let mut tracker = BoilerplateTracker::new();

        for _ in 0..3 {
            tracker.record_page("a.com", &["Shared text"]);
            tracker.record_page("b.com", &["Shared text"]);
        }

        // Mark as boilerplate for a.com only, clear b.com.
        tracker.clear_domain("b.com");

        assert!(tracker.is_boilerplate("a.com", "Shared text"));
        assert!(!tracker.is_boilerplate("b.com", "Shared text"));
    }

    #[test]
    fn test_empty_blocks_ignored() {
        let mut tracker = BoilerplateTracker::new();
        let domain = "example.com";

        for _ in 0..5 {
            tracker.record_page(domain, &["", "   "]);
        }

        assert_eq!(tracker.boilerplate_count(domain), 0);
    }

    #[test]
    fn test_large_blocks_ignored() {
        let mut tracker = BoilerplateTracker::new();
        let domain = "example.com";
        let big_block = "x".repeat(2001);

        for _ in 0..5 {
            tracker.record_page(domain, &[&big_block]);
        }

        assert_eq!(tracker.boilerplate_count(domain), 0);
    }
}
