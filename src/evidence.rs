use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{BlockType, Citation, Confidence, Evidence};

/// Compute SHA256 hex digest of content.
pub fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Calculate confidence based on content length and block detection.
///
/// - High: content >500 chars and no block
/// - Medium: content >100 chars (or blocked but some content)
/// - Low: otherwise
pub fn calculate_confidence(
    content_length: usize,
    block_detected: Option<&BlockType>,
) -> Confidence {
    match (content_length, block_detected) {
        (len, None) if len > 500 => Confidence::High,
        (len, Some(_)) if len > 500 => Confidence::Medium,
        (len, _) if len > 100 => Confidence::Medium,
        _ => Confidence::Low,
    }
}

/// Extract markdown-style citations from content.
///
/// Parses `[text](url)` patterns and returns Citation structs.
/// Skips empty text/urls and relative urls (no scheme).
pub fn extract_citations(content: &str, _source_url: &str) -> Vec<Citation> {
    let mut citations = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (i, c) in content.char_indices() {
        if c == '[' {
            // Find matching ]
            let bracket_start = i;
            let mut depth = 1i32;
            let mut text_end = None;
            for (j, c2) in content.char_indices().skip(i + 1) {
                match c2 {
                    '[' => depth += 1,
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            text_end = Some(j);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let Some(text_end_idx) = text_end else {
                continue;
            };

            // Check for ( right after ]
            let rest = &content[text_end_idx + 1..];
            let mut url_chars = rest.char_indices();
            let Some((_, '(')) = url_chars.next() else {
                continue;
            };

            // Read until )
            let mut url_buf = String::new();
            let mut url_end = None;
            for (offset, c3) in url_chars {
                if c3 == ')' {
                    url_end = Some(text_end_idx + 1 + offset);
                    break;
                }
                url_buf.push(c3);
            }
            let Some(_) = url_end else { continue };

            let link_text = content[bracket_start + 1..text_end_idx].trim().to_string();
            let link_url = url_buf.trim().to_string();

            // Skip empty or obviously malformed
            if link_text.is_empty() || link_url.is_empty() {
                continue;
            }
            // Only include http(s) urls
            if !link_url.starts_with("http://") && !link_url.starts_with("https://") {
                continue;
            }

            // Deduplicate by url
            if seen.contains(&link_url) {
                continue;
            }
            seen.insert(link_url.clone());

            // Build context: 40 chars around the citation
            let ctx_start = bracket_start.saturating_sub(40);
            let ctx_end = (bracket_start + 60).min(content.len());
            let context = content[ctx_start..ctx_end]
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();

            citations.push(Citation {
                text: link_text,
                source_url: link_url,
                context,
            });

            // Cap at 50 citations for performance
            if citations.len() >= 50 {
                break;
            }
        }
    }
    citations
}

/// Build an Evidence bundle from a successful fetch.
///
/// Returns a fully populated Evidence struct with SHA256 hash,
/// confidence score, extracted citations, and ISO timestamp.
pub fn build_evidence(
    content: &str,
    url: &str,
    strategy: &str,
    content_length: usize,
    block_detected: Option<&BlockType>,
) -> Evidence {
    let content_hash = compute_hash(content);
    let confidence = calculate_confidence(content_length, block_detected);
    let citations = extract_citations(content, url);

    let retrieved_at = {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Format as ISO 8601 UTC
        chrono_compat::secs_to_iso(secs)
    };

    Evidence {
        url: url.to_string(),
        retrieved_at,
        content_hash,
        strategy: strategy.to_string(),
        confidence,
        citations,
    }
}

/// Format the evidence as a one-line prefix for MCP responses.
///
/// Example: `📎 Evidence: sha256:abc123… | fetched 2026-05-17T14:30:00Z | confidence: high | 3 citations`
pub fn format_evidence_line(evidence: &Evidence) -> String {
    let hash_preview = &evidence.content_hash[..16.min(evidence.content_hash.len())];
    let conf = match evidence.confidence {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    };
    format!(
        "📎 Evidence: sha256:{}… | fetched {} | confidence: {} | {} citations",
        hash_preview,
        evidence.retrieved_at,
        conf,
        evidence.citations.len()
    )
}

/// Store evidence in the database.
pub fn store_evidence(
    storage: &crate::storage::Storage,
    evidence: &Evidence,
    diagnostics_json: Option<&str>,
) -> anyhow::Result<()> {
    let citations_json = if evidence.citations.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&evidence.citations)?)
    };
    storage.store_evidence(
        &evidence.url,
        &evidence.content_hash,
        &evidence.strategy,
        &evidence.retrieved_at,
        match evidence.confidence {
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        },
        citations_json.as_deref(),
        diagnostics_json,
    )
}

// Lightweight chrono-free ISO timestamp from epoch seconds
mod chrono_compat {
    pub fn secs_to_iso(secs: u64) -> String {
        // Simple datetime calculation without chrono
        let days_since_epoch = secs / 86400;
        let time_of_day = secs % 86400;
        let hours = time_of_day / 3600;
        let minutes = (time_of_day % 3600) / 60;
        let seconds = time_of_day % 60;

        // Calculate year/month/day from days since epoch
        let (year, month, day) = days_to_ymd(days_since_epoch);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds
        )
    }

    fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
        let mut year = 1970u64;
        loop {
            let days_in_year = if is_leap(year) { 366 } else { 365 };
            if days < days_in_year {
                break;
            }
            days -= days_in_year;
            year += 1;
        }
        let leap = is_leap(year);
        let month_days: [u64; 12] = if leap {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };
        let mut month = 0u64;
        for (i, &md) in month_days.iter().enumerate() {
            if days < md {
                month = i as u64 + 1;
                break;
            }
            days -= md;
        }
        if month == 0 {
            month = 12;
        }
        (year, month, days + 1)
    }

    fn is_leap(year: u64) -> bool {
        (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_hash_deterministic() {
        let content = "Hello, world!";
        let h1 = compute_hash(content);
        let h2 = compute_hash(content);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA256 hex = 64 chars
    }

    #[test]
    fn test_compute_hash_different_inputs() {
        let h1 = compute_hash("foo");
        let h2 = compute_hash("bar");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_compute_hash_empty() {
        let h = compute_hash("");
        // Known SHA256 of empty string
        assert_eq!(
            h,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_confidence_high_no_block() {
        assert_eq!(calculate_confidence(501, None), Confidence::High);
    }

    #[test]
    fn test_confidence_medium_with_block() {
        assert_eq!(
            calculate_confidence(501, Some(&BlockType::Cloudflare)),
            Confidence::Medium
        );
    }

    #[test]
    fn test_confidence_medium_short_content() {
        assert_eq!(calculate_confidence(200, None), Confidence::Medium);
    }

    #[test]
    fn test_confidence_low() {
        assert_eq!(calculate_confidence(50, None), Confidence::Low);
    }

    #[test]
    fn test_confidence_low_blocked_tiny_content() {
        assert_eq!(
            calculate_confidence(50, Some(&BlockType::RateLimit)),
            Confidence::Low
        );
    }

    #[test]
    fn test_confidence_medium_blocked_short() {
        assert_eq!(
            calculate_confidence(200, Some(&BlockType::Captcha)),
            Confidence::Medium
        );
    }

    #[test]
    fn test_extract_citations_basic() {
        let content = "Check out [Rust](https://rust-lang.org) for systems programming.";
        let citations = extract_citations(content, "https://example.com");
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].text, "Rust");
        assert_eq!(citations[0].source_url, "https://rust-lang.org");
    }

    #[test]
    fn test_extract_citations_multiple() {
        let content = "[A](https://a.com) and [B](https://b.com) and [C](https://c.com)";
        let citations = extract_citations(content, "https://example.com");
        assert_eq!(citations.len(), 3);
    }

    #[test]
    fn test_extract_citations_dedup() {
        let content = "[First](https://example.com) and [Second](https://example.com)";
        let citations = extract_citations(content, "https://example.com");
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].text, "First");
    }

    #[test]
    fn test_extract_citations_skip_relative() {
        let content = "[Link](/relative/path) is skipped";
        let citations = extract_citations(content, "https://example.com");
        assert!(citations.is_empty());
    }

    #[test]
    fn test_extract_citations_empty_text() {
        let content = "[](https://example.com)";
        let citations = extract_citations(content, "https://example.com");
        assert!(citations.is_empty());
    }

    #[test]
    fn test_extract_citations_no_links() {
        let content = "Just plain text, no links here.";
        let citations = extract_citations(content, "https://example.com");
        assert!(citations.is_empty());
    }

    #[test]
    fn test_extract_citations_context() {
        let content = "Some preceding text [Link](https://example.com) some trailing text";
        let citations = extract_citations(content, "https://source.com");
        assert_eq!(citations.len(), 1);
        assert!(citations[0].context.contains("Link"));
    }

    #[test]
    fn test_build_evidence_basic() {
        let content = "A".repeat(600);
        let evidence = build_evidence(&content, "https://example.com", "HttpSimple", 600, None);
        assert_eq!(evidence.url, "https://example.com");
        assert_eq!(evidence.strategy, "HttpSimple");
        assert_eq!(evidence.confidence, Confidence::High);
        assert_eq!(evidence.content_hash.len(), 64);
        assert!(evidence.retrieved_at.ends_with('Z'));
    }

    #[test]
    fn test_build_evidence_with_citations() {
        let content = "[Rust](https://rust-lang.org) and [Docs](https://doc.rust-lang.org)";
        let evidence = build_evidence(
            content,
            "https://example.com",
            "HttpSimple",
            content.len(),
            None,
        );
        // Confidence: content length < 100 but no block → Low
        // Actually content.len() is the 3rd param, let's check:
        // content is ~65 chars, so confidence = Low
        // Wait the test passes content.len() which is ~65 chars < 100 → Low
        // But we want to verify citations are extracted
        assert_eq!(evidence.citations.len(), 2);
    }

    #[test]
    fn test_format_evidence_line() {
        let evidence = Evidence {
            url: "https://example.com".to_string(),
            retrieved_at: "2026-05-17T14:30:00Z".to_string(),
            content_hash: "abc123def456abc123def456abc123def456abc123def456abc123def456abcd"
                .to_string(),
            strategy: "HttpSimple".to_string(),
            confidence: Confidence::High,
            citations: vec![Citation {
                text: "A".to_string(),
                source_url: "https://a.com".to_string(),
                context: String::new(),
            }],
        };
        let line = format_evidence_line(&evidence);
        assert!(line.starts_with("📎 Evidence: sha256:abc123def456abc1"));
        assert!(line.contains("confidence: high"));
        assert!(line.contains("1 citations"));
        assert!(line.contains("fetched 2026-05-17T14:30:00Z"));
    }

    #[test]
    fn test_chrono_compat_iso_format() {
        // 0 epoch = 1970-01-01T00:00:00Z
        assert_eq!(chrono_compat::secs_to_iso(0), "1970-01-01T00:00:00Z");
        // Known timestamp: 1700000000 = 2023-11-14T22:13:20Z
        let result = chrono_compat::secs_to_iso(1_700_000_000);
        assert!(result.starts_with("2023-"));
        assert!(result.ends_with('Z'));
    }

    #[test]
    fn test_store_evidence_integration() {
        let storage = crate::storage::Storage::open(":memory:").unwrap();
        let evidence = Evidence {
            url: "https://example.com".to_string(),
            retrieved_at: "2026-05-17T14:30:00Z".to_string(),
            content_hash: "abc123".to_string(),
            strategy: "HttpSimple".to_string(),
            confidence: Confidence::High,
            citations: vec![],
        };
        store_evidence(&storage, &evidence, None).unwrap();

        let row = storage.get_evidence("https://example.com").unwrap();
        assert!(row.is_some());
        let (hash, strategy, confidence, retrieved_at) = row.unwrap();
        assert_eq!(hash, "abc123");
        assert_eq!(strategy, "HttpSimple");
        assert_eq!(confidence, "high");
        assert_eq!(retrieved_at, "2026-05-17T14:30:00Z");
    }

    #[test]
    fn test_store_evidence_with_citations() {
        let storage = crate::storage::Storage::open(":memory:").unwrap();
        let evidence = Evidence {
            url: "https://example.com".to_string(),
            retrieved_at: "2026-05-17T14:30:00Z".to_string(),
            content_hash: "def456".to_string(),
            strategy: "TlsChrome".to_string(),
            confidence: Confidence::Medium,
            citations: vec![Citation {
                text: "Rust".to_string(),
                source_url: "https://rust-lang.org".to_string(),
                context: "See [Rust]".to_string(),
            }],
        };
        store_evidence(&storage, &evidence, Some("{\"test\":true}")).unwrap();

        let row = storage.get_evidence("https://example.com").unwrap();
        assert!(row.is_some());
        let (_, _, confidence, _) = row.unwrap();
        assert_eq!(confidence, "medium");
    }
}
