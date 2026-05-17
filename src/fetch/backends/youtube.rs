use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::types::{FetchError, FetchRequest, FetchResponse, Strategy};

use crate::fetch::backend::FetchBackend;

// ---------------------------------------------------------------------------
// URL detection helpers
// ---------------------------------------------------------------------------

fn extract_host(url: &str) -> &str {
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
}

/// Returns `true` for YouTube video page URLs (watch, short-link, embed).
pub fn is_youtube_url(url: &str) -> bool {
    let host = extract_host(url).to_lowercase();
    if ![
        "youtube.com",
        "www.youtube.com",
        "youtu.be",
        "m.youtube.com",
    ]
    .contains(&host.as_str())
    {
        return false;
    }
    // For youtu.be short URLs, the path itself contains the video ID.
    if host == "youtu.be" {
        return true;
    }
    // For full URLs we only handle /watch and /embed paths.
    let path = extract_path(url);
    path.starts_with("/watch") || path.starts_with("/embed/")
}

/// Extract the 11-character YouTube video ID from a URL.
pub fn extract_video_id(url: &str) -> Option<String> {
    let host = extract_host(url).to_lowercase();

    if host == "youtu.be" {
        // e.g. https://youtu.be/dQw4w9WgXcQ
        let path = extract_path(url).trim_start_matches('/');
        let id = path.split('?').next().unwrap_or(path);
        return validate_id(id);
    }

    let path = extract_path(url);

    // /watch?v=...&other=...
    if path.starts_with("/watch") {
        let query = path.split('?').nth(1)?;
        for pair in query.split('&') {
            let mut kv = pair.splitn(2, '=');
            if kv.next()? == "v" {
                return validate_id(kv.next()?);
            }
        }
    }

    // /embed/VIDEO_ID
    if path.starts_with("/embed/") {
        let id = path.trim_start_matches("/embed/").split('?').next()?;
        return validate_id(id);
    }

    None
}

fn validate_id(id: &str) -> Option<String> {
    if id.len() >= 11
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        Some(id[..11].to_string())
    } else {
        None
    }
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

// ---------------------------------------------------------------------------
// yt-dlp helpers
// ---------------------------------------------------------------------------

/// Check whether `yt-dlp` is available on `$PATH`.
fn is_ytdlp_available() -> bool {
    std::process::Command::new("yt-dlp")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Call `yt-dlp --dump-json <url>` and return the parsed JSON object.
fn ytdlp_dump_json(url: &str, _timeout: Duration) -> Result<serde_json::Value, FetchError> {
    let output = std::process::Command::new("yt-dlp")
        .args(["--dump-json", "--no-playlist", url])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| FetchError::NetworkError(format!("yt-dlp dump-json failed: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(FetchError::NetworkError(format!(
            "yt-dlp dump-json error: {stderr}"
        )));
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|e| FetchError::NetworkError(format!("yt-dlp JSON parse error: {e}")))
}

/// Download subtitles via yt-dlp and return the VTT content.
fn ytdlp_fetch_subtitles(
    url: &str,
    video_id: &str,
    _timeout: Duration,
) -> Result<Option<String>, FetchError> {
    let tmp_dir = std::env::temp_dir().join(format!("kaelo-yt-{video_id}"));
    let _ = std::fs::create_dir_all(&tmp_dir);

    let output_template = tmp_dir.join("sub").to_string_lossy().to_string();

    let output = std::process::Command::new("yt-dlp")
        .args([
            "--write-sub",
            "--write-auto-sub",
            "--sub-lang",
            "en,fr",
            "--skip-download",
            "--no-playlist",
            "--output",
            &output_template,
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| FetchError::NetworkError(format!("yt-dlp subtitle fetch failed: {e}")))?;

    if !output.status.success() {
        // Subtitles may simply not exist — not a hard error.
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Ok(None);
    }

    // Find any .vtt or .srt file in tmp_dir
    let subtitle_content = std::fs::read_dir(&tmp_dir).ok().and_then(|entries| {
        entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "vtt" || ext == "srt")
                    .unwrap_or(false)
            })
            .find(|_| true)
            .and_then(|e| std::fs::read_to_string(e.path()).ok())
    });

    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok(subtitle_content)
}

// ---------------------------------------------------------------------------
// VTT / SRT parsing
// ---------------------------------------------------------------------------

/// Parse WebVTT content into a list of `(timestamp_str, text)` pairs.
pub fn parse_vtt(content: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut current_ts = String::new();
    let mut current_text = String::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip headers and empty lines
        if trimmed.starts_with("WEBVTT")
            || trimmed.starts_with("Kind:")
            || trimmed.starts_with("Language:")
            || trimmed.is_empty()
        {
            // Flush accumulated text
            if !current_text.is_empty() && !current_ts.is_empty() {
                let text = current_text.trim().to_string();
                if !text.is_empty() {
                    entries.push((current_ts.clone(), text));
                }
                current_text.clear();
            }
            current_ts.clear();
            continue;
        }

        // Timestamp line: 00:00:01.234 --> 00:00:05.678
        if trimmed.contains("-->") {
            // Flush previous entry
            if !current_text.is_empty() && !current_ts.is_empty() {
                let text = current_text.trim().to_string();
                if !text.is_empty() {
                    entries.push((current_ts.clone(), text));
                }
                current_text.clear();
            }
            // Take the start timestamp
            current_ts = trimmed.split("-->").next().unwrap_or("").trim().to_string();
            continue;
        }

        // Text line — strip any HTML/VTT tags
        let clean = strip_vtt_tags(trimmed);
        if !clean.is_empty() {
            if !current_text.is_empty() {
                current_text.push(' ');
            }
            current_text.push_str(&clean);
        }
    }

    // Flush last entry
    if !current_text.is_empty() && !current_ts.is_empty() {
        let text = current_text.trim().to_string();
        if !text.is_empty() {
            entries.push((current_ts.clone(), text));
        }
    }

    entries
}

/// Parse SRT content into a list of `(timestamp_str, text)` pairs.
pub fn parse_srt(content: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let blocks: Vec<&str> = content.split("\n\n").collect();

    for block in blocks {
        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 3 {
            continue;
        }
        // Line 0 = sequence number, Line 1 = timestamp, Line 2+ = text
        let ts_line = lines[1].trim();
        let start_ts = ts_line.split("-->").next().unwrap_or("").trim();
        let text: String = lines[2..]
            .iter()
            .map(|l| strip_vtt_tags(l.trim()))
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            entries.push((start_ts.to_string(), text));
        }
    }

    entries
}

fn strip_vtt_tags(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// Convert timestamp like "00:01:23.456" to "[1:23]" format.
fn format_timestamp(ts: &str) -> String {
    let parts: Vec<&str> = ts.split(':').collect();
    if parts.len() < 3 {
        return ts.to_string();
    }
    let hours: u64 = parts[0].parse().unwrap_or(0);
    let mins: u64 = parts[1].parse().unwrap_or(0);
    let secs = parts[2].split('.').next().unwrap_or("0");

    if hours > 0 {
        format!("[{hours}:{mins:02}:{secs}]")
    } else {
        format!("[{mins}:{secs}]")
    }
}

// ---------------------------------------------------------------------------
// Markdown formatting
// ---------------------------------------------------------------------------

fn format_transcript_markdown(
    title: &str,
    duration: Option<u64>,
    description: Option<&str>,
    entries: &[(String, String)],
) -> String {
    let mut md = String::new();

    md.push_str(&format!("# {title}\n\n"));

    if let Some(dur) = duration {
        let mins = dur / 60;
        let secs = dur % 60;
        md.push_str(&format!("**Duration:** {mins}:{secs:02}\n\n"));
    }

    md.push_str("---\n\n");

    if entries.is_empty() {
        md.push_str("*No subtitles available.*\n\n");
        if let Some(desc) = description {
            if !desc.is_empty() {
                md.push_str("## Description\n\n");
                md.push_str(desc);
                md.push('\n');
            }
        }
    } else {
        md.push_str("## Transcript\n\n");
        for (ts, text) in entries {
            md.push_str(&format!("**{}** {text}\n\n", format_timestamp(ts)));
        }
    }

    md
}

// ---------------------------------------------------------------------------
// Backend implementation
// ---------------------------------------------------------------------------

pub struct YouTubeBackend;

impl YouTubeBackend {
    pub fn new() -> Result<Self, FetchError> {
        Ok(Self)
    }
}

impl FetchBackend for YouTubeBackend {
    async fn fetch(&self, request: FetchRequest) -> Result<FetchResponse, FetchError> {
        let start = Instant::now();
        let url = &request.url;

        // 1. Must be a YouTube URL
        if !is_youtube_url(url) {
            return Err(FetchError::NetworkError(format!(
                "Not a YouTube URL: {url}"
            )));
        }

        let video_id = extract_video_id(url).ok_or_else(|| {
            FetchError::NetworkError(format!("Cannot extract video ID from: {url}"))
        })?;

        // 2. Check yt-dlp availability
        if !is_ytdlp_available() {
            // Fall back to PublicApi (noembed) for basic metadata
            tracing::info!("yt-dlp not found, falling back to noembed for YouTube URL");
            return Err(FetchError::NetworkError("yt-dlp not available".to_string()));
        }

        // 3. Fetch metadata via yt-dlp --dump-json
        let metadata = ytdlp_dump_json(url, request.timeout)?;

        let title = metadata
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(untitled)")
            .to_string();

        let duration = metadata
            .get("duration")
            .and_then(|v| v.as_f64())
            .map(|d| d as u64);

        let description = metadata.get("description").and_then(|v| v.as_str());

        // 4. Try fetching subtitles
        let subtitle_raw = ytdlp_fetch_subtitles(url, &video_id, request.timeout)?;

        let entries = match subtitle_raw {
            Some(raw) if raw.trim_start().starts_with("WEBVTT") => parse_vtt(&raw),
            Some(raw) if raw.contains("-->") => parse_srt(&raw),
            Some(_) => Vec::new(),
            None => Vec::new(),
        };

        // 5. Format as Markdown
        let markdown = format_transcript_markdown(&title, duration, description, &entries);

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
        Strategy::YouTube
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── URL detection ──────────────────────────────────────────────────

    #[test]
    fn test_is_youtube_url_watch() {
        assert!(is_youtube_url(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        ));
        assert!(is_youtube_url("https://youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(is_youtube_url("https://m.youtube.com/watch?v=dQw4w9WgXcQ"));
    }

    #[test]
    fn test_is_youtube_url_short() {
        assert!(is_youtube_url("https://youtu.be/dQw4w9WgXcQ"));
        assert!(is_youtube_url("http://youtu.be/dQw4w9WgXcQ?t=42"));
    }

    #[test]
    fn test_is_youtube_url_embed() {
        assert!(is_youtube_url("https://www.youtube.com/embed/dQw4w9WgXcQ"));
    }

    #[test]
    fn test_is_youtube_url_rejects_non_video() {
        assert!(!is_youtube_url("https://www.youtube.com/"));
        assert!(!is_youtube_url(
            "https://www.youtube.com/results?search_query=rust"
        ));
        assert!(!is_youtube_url("https://vimeo.com/123456"));
        assert!(!is_youtube_url("https://example.com/watch?v=abc"));
    }

    // ── Video ID extraction ────────────────────────────────────────────

    #[test]
    fn test_extract_video_id_watch() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_video_id("https://youtube.com/watch?v=dQw4w9WgXcQ&list=PLxyz&t=42"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn test_extract_video_id_short() {
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ?t=42"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn test_extract_video_id_embed() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/embed/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn test_extract_video_id_mobile() {
        assert_eq!(
            extract_video_id("https://m.youtube.com/watch?v=abc123XYZ_0"),
            Some("abc123XYZ_0".to_string())
        );
    }

    #[test]
    fn test_extract_video_id_invalid() {
        assert_eq!(extract_video_id("https://www.youtube.com/"), None);
        assert_eq!(
            extract_video_id("https://www.youtube.com/results?search_query=rust"),
            None
        );
    }

    // ── VTT parsing ────────────────────────────────────────────────────

    #[test]
    fn test_parse_vtt_basic() {
        let vtt = "\
WEBVTT

00:00:01.000 --> 00:00:04.000
Hello, world!

00:00:05.000 --> 00:00:08.000
This is a test.";

        let entries = parse_vtt(vtt);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "00:00:01.000");
        assert_eq!(entries[0].1, "Hello, world!");
        assert_eq!(entries[1].0, "00:00:05.000");
        assert_eq!(entries[1].1, "This is a test.");
    }

    #[test]
    fn test_parse_vtt_multiline() {
        let vtt = "\
WEBVTT

00:00:01.000 --> 00:00:04.000
First line
second line

00:00:05.000 --> 00:00:08.000
Third block";

        let entries = parse_vtt(vtt);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].1, "First line second line");
    }

    #[test]
    fn test_parse_vtt_strips_tags() {
        let vtt = "\
WEBVTT

00:00:01.000 --> 00:00:04.000
<b>Bold text</b> and normal";

        let entries = parse_vtt(vtt);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].1, "Bold text and normal");
    }

    #[test]
    fn test_parse_vtt_empty() {
        let vtt = "WEBVTT\n\n";
        let entries = parse_vtt(vtt);
        assert!(entries.is_empty());
    }

    // ── SRT parsing ────────────────────────────────────────────────────

    #[test]
    fn test_parse_srt_basic() {
        let srt = "\
1
00:00:01,000 --> 00:00:04,000
Hello, world!

2
00:00:05,000 --> 00:00:08,000
This is a test.";

        let entries = parse_srt(srt);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "00:00:01,000");
        assert_eq!(entries[0].1, "Hello, world!");
        assert_eq!(entries[1].0, "00:00:05,000");
        assert_eq!(entries[1].1, "This is a test.");
    }

    // ── Timestamp formatting ───────────────────────────────────────────

    #[test]
    fn test_format_timestamp_short() {
        assert_eq!(format_timestamp("00:01:23.456"), "[1:23]");
    }

    #[test]
    fn test_format_timestamp_with_hours() {
        assert_eq!(format_timestamp("01:23:45.000"), "[1:23:45]");
    }

    // ── Markdown output ────────────────────────────────────────────────

    #[test]
    fn test_format_transcript_with_subtitles() {
        let entries = vec![
            ("00:00:01.000".to_string(), "Hello".to_string()),
            ("00:00:05.000".to_string(), "World".to_string()),
        ];
        let md =
            format_transcript_markdown("Test Video", Some(120), Some("A description"), &entries);
        assert!(md.contains("# Test Video"));
        assert!(md.contains("**Duration:** 2:00"));
        assert!(md.contains("## Transcript"));
        assert!(md.contains("**[0:01]** Hello"));
        assert!(md.contains("**[0:05]** World"));
        assert!(!md.contains("## Description")); // subtitles take precedence
    }

    #[test]
    fn test_format_transcript_no_subtitles() {
        let md = format_transcript_markdown("Test Video", None, Some("Fallback description"), &[]);
        assert!(md.contains("*No subtitles available.*"));
        assert!(md.contains("## Description"));
        assert!(md.contains("Fallback description"));
    }

    // ── Backend name ───────────────────────────────────────────────────

    #[test]
    fn test_backend_name() {
        let backend = YouTubeBackend::new().expect("backend creation");
        assert_eq!(backend.name(), Strategy::YouTube);
    }

    #[test]
    fn test_backend_creation() {
        assert!(YouTubeBackend::new().is_ok());
    }
}
