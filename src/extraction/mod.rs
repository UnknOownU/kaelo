//! Content extraction pipeline: HTML → readability → clean Markdown.
//!
//! Uses dom_smoothie to extract the main content from web pages,
//! stripping navigation, headers, footers, and other boilerplate.

pub mod boilerplate;
pub mod markdown;
pub mod quality;
pub mod scoring;
pub mod spa_detect;
pub mod structural;
pub mod token_estimator;

use crate::types::ExtractionMode;

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

    if ct_lower.contains("application/json") {
        return extract_json(html, url, content_type);
    }

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

/// Extract content using the specified extraction mode.
///
/// Dispatches to mode-specific extraction logic. `Markdown` mode delegates to
/// the existing [`extract`] function; all other modes perform their own HTML
/// processing using manual string scanning (no additional crate dependencies).
pub fn extract_with_mode(
    html: &str,
    url: &str,
    content_type: &str,
    mode: &ExtractionMode,
) -> anyhow::Result<Option<ExtractedContent>> {
    match mode {
        ExtractionMode::Markdown => extract(html, url, content_type),
        ExtractionMode::Text => extract_text_mode(html, url, content_type),
        ExtractionMode::Html => extract_html_mode(html, url, content_type),
        ExtractionMode::Links => extract_links_mode(html, url, content_type),
        ExtractionMode::Metadata => extract_metadata_mode(html, url, content_type),
        ExtractionMode::JsonLd => extract_jsonld_mode(html, url, content_type),
        ExtractionMode::Tables => extract_tables_mode(html, url, content_type),
    }
}

fn extract_text_mode(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let title = extract_title_tag(html).unwrap_or_else(|| extract_title_from_url(url));
    let cleaned = strip_element_contents(html, &["script", "style"]);
    let text = strip_all_tags(&cleaned);
    let text = collapse_whitespace(&text);

    if text.is_empty() {
        return Ok(None);
    }

    Ok(Some(ExtractedContent {
        title,
        content: html.to_string(),
        text_content: text.clone(),
        url: url.to_string(),
        length: text.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_html_mode(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let title = extract_title_tag(html).unwrap_or_else(|| extract_title_from_url(url));
    let cleaned = strip_element_contents(
        html,
        &[
            "script", "style", "nav", "footer", "header", "noscript", "iframe", "svg",
        ],
    );
    let cleaned = strip_html_comments(&cleaned);
    let trimmed = cleaned.trim().to_string();

    if trimmed.is_empty() {
        return Ok(None);
    }

    Ok(Some(ExtractedContent {
        title,
        content: trimmed.clone(),
        text_content: trimmed.clone(),
        url: url.to_string(),
        length: trimmed.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_links_mode(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let title = extract_title_tag(html).unwrap_or_else(|| extract_title_from_url(url));
    let links = extract_anchor_tags(html);

    let text_content = if links.is_empty() {
        "No links found on this page.".to_string()
    } else {
        links
            .iter()
            .map(|(text, href)| format!("- [{text}]({href})"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(Some(ExtractedContent {
        title,
        content: html.to_string(),
        text_content: text_content.clone(),
        url: url.to_string(),
        length: text_content.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_metadata_mode(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let title = extract_title_tag(html).unwrap_or_else(|| extract_title_from_url(url));
    let mut entries: Vec<(String, String)> = Vec::new();

    if let Some(t) = extract_title_tag(html) {
        entries.push(("title".to_string(), t));
    }

    extract_meta_by_name(html, "description", &mut entries);
    extract_meta_by_name(html, "author", &mut entries);
    extract_meta_by_name(html, "keywords", &mut entries);
    extract_og_tags(html, &mut entries);
    extract_canonical(html, &mut entries);
    extract_time_tags(html, &mut entries);

    let text_content = if entries.is_empty() {
        "No metadata found on this page.".to_string()
    } else {
        entries
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(Some(ExtractedContent {
        title,
        content: html.to_string(),
        text_content: text_content.clone(),
        url: url.to_string(),
        length: text_content.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_jsonld_mode(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let title = extract_title_tag(html).unwrap_or_else(|| extract_title_from_url(url));
    let blocks = extract_jsonld_blocks(html);

    let text_content = if blocks.is_empty() {
        "No structured data (JSON-LD) found on this page.".to_string()
    } else if blocks.len() == 1 {
        blocks.into_iter().next().unwrap()
    } else {
        blocks
            .into_iter()
            .enumerate()
            .map(|(i, block)| {
                if i == 0 {
                    format!("### Structured Data Block 1\n{block}")
                } else {
                    format!("\n### Structured Data Block {}\n{block}", i + 1)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(Some(ExtractedContent {
        title,
        content: html.to_string(),
        text_content: text_content.clone(),
        url: url.to_string(),
        length: text_content.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_jsonld_blocks(html: &str) -> Vec<String> {
    let lower = html.to_lowercase();
    let mut blocks = Vec::new();
    let mut pos = 0;

    while pos < html.len() {
        let Some(script_start) = lower[pos..].find("<script") else {
            break;
        };
        let abs_script = pos + script_start;

        let Some(gt) = lower[abs_script..].find('>') else {
            break;
        };
        let tag_end = abs_script + gt;
        let tag_content = &lower[abs_script..tag_end];

        if !tag_content.contains("application/ld+json") {
            pos = tag_end + 1;
            continue;
        }

        let content_start = tag_end + 1;
        let Some(close) = lower[content_start..].find("</script>") else {
            break;
        };
        let json_str = html[content_start..content_start + close].trim();

        if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Ok(pretty) = serde_json::to_string_pretty(&val) {
                blocks.push(pretty);
            } else {
                blocks.push(json_str.to_string());
            }
        } else {
            blocks.push(json_str.to_string());
        }

        pos = content_start + close + 9;
    }

    blocks
}

fn extract_tables_mode(
    html: &str,
    url: &str,
    content_type: &str,
) -> anyhow::Result<Option<ExtractedContent>> {
    let title = extract_title_tag(html).unwrap_or_else(|| extract_title_from_url(url));
    let tables = extract_html_tables(html);

    let text_content = if tables.is_empty() {
        "No tables found on this page.".to_string()
    } else if tables.len() == 1 {
        tables.into_iter().next().unwrap()
    } else {
        tables
            .into_iter()
            .enumerate()
            .map(|(i, table)| {
                if i == 0 {
                    format!("### Table 1\n{table}")
                } else {
                    format!("\n### Table {}\n{table}", i + 1)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    Ok(Some(ExtractedContent {
        title,
        content: html.to_string(),
        text_content: text_content.clone(),
        url: url.to_string(),
        length: text_content.len(),
        content_type: content_type.to_string(),
    }))
}

fn extract_html_tables(html: &str) -> Vec<String> {
    let lower = html.to_lowercase();
    let mut results = Vec::new();
    let mut pos = 0;

    while pos < html.len() {
        let Some(table_start) = lower[pos..].find("<table") else {
            break;
        };
        let abs_start = pos + table_start;

        let Some(gt) = lower[abs_start..].find('>') else {
            break;
        };
        let content_start = abs_start + gt + 1;

        let Some(close) = lower[content_start..].find("</table>") else {
            break;
        };
        let table_html = &html[content_start..content_start + close];

        let rows = extract_table_rows(table_html);
        if rows.is_empty() {
            pos = content_start + close + 8;
            continue;
        }

        let (header, data_rows) = if rows.iter().all(|cell| cell.0) {
            let header_cells: Vec<String> = rows.iter().map(|c| c.1.clone()).collect();
            let remaining = extract_table_data_rows(table_html);
            (header_cells, remaining)
        } else {
            let header_cells: Vec<String> = rows.iter().map(|c| c.1.clone()).collect();
            let remaining = extract_table_data_rows_no_header(table_html);
            (header_cells, remaining)
        };

        if header.is_empty() {
            pos = content_start + close + 8;
            continue;
        }

        let separator: Vec<String> = header.iter().map(|_| "---".to_string()).collect();

        let mut md = format!("| {} |", header.join(" | "));
        md.push_str(&format!("\n| {} |", separator.join(" | ")));

        for row in &data_rows {
            md.push_str(&format!("\n| {} |", row.join(" | ")));
        }

        results.push(md);
        pos = content_start + close + 8;
    }

    results
}

fn extract_table_rows(table_html: &str) -> Vec<(bool, String)> {
    let lower = table_html.to_lowercase();
    let mut cells = Vec::new();
    let mut pos = 0;

    while pos < table_html.len() {
        let Some(tr_start) = lower[pos..].find("<tr") else {
            break;
        };
        let abs_tr = pos + tr_start;

        let Some(gt) = lower[abs_tr..].find('>') else {
            break;
        };
        let row_start = abs_tr + gt + 1;

        let Some(tr_close) = lower[row_start..].find("</tr>") else {
            break;
        };
        let row_html = &table_html[row_start..row_start + tr_close];

        let row_cells = extract_row_cells(row_html);
        if !row_cells.is_empty() {
            cells = row_cells;
            break;
        }
        pos = row_start + tr_close + 5;
    }

    cells
}

fn extract_table_data_rows(table_html: &str) -> Vec<Vec<String>> {
    let lower = table_html.to_lowercase();
    let mut rows = Vec::new();
    let mut pos = 0;
    let mut is_first_row = true;

    while pos < table_html.len() {
        let Some(tr_start) = lower[pos..].find("<tr") else {
            break;
        };
        let abs_tr = pos + tr_start;

        let Some(gt) = lower[abs_tr..].find('>') else {
            break;
        };
        let row_start = abs_tr + gt + 1;

        let Some(tr_close) = lower[row_start..].find("</tr>") else {
            break;
        };
        let row_html = &table_html[row_start..row_start + tr_close];

        let cells = extract_row_cells(row_html);

        if is_first_row {
            is_first_row = false;
            if cells.iter().all(|(is_th, _)| *is_th) {
                pos = row_start + tr_close + 5;
                continue;
            }
        }

        let data: Vec<String> = cells.iter().map(|(_, text)| text.clone()).collect();
        if !data.is_empty() {
            rows.push(data);
        }

        pos = row_start + tr_close + 5;
    }

    rows
}

fn extract_table_data_rows_no_header(table_html: &str) -> Vec<Vec<String>> {
    let lower = table_html.to_lowercase();
    let mut rows = Vec::new();
    let mut pos = 0;
    let mut is_first_row = true;

    while pos < table_html.len() {
        let Some(tr_start) = lower[pos..].find("<tr") else {
            break;
        };
        let abs_tr = pos + tr_start;

        let Some(gt) = lower[abs_tr..].find('>') else {
            break;
        };
        let row_start = abs_tr + gt + 1;

        let Some(tr_close) = lower[row_start..].find("</tr>") else {
            break;
        };
        let row_html = &table_html[row_start..row_start + tr_close];

        if is_first_row {
            is_first_row = false;
            pos = row_start + tr_close + 5;
            continue;
        }

        let cells = extract_row_cells(row_html);
        let data: Vec<String> = cells.iter().map(|(_, text)| text.clone()).collect();
        if !data.is_empty() {
            rows.push(data);
        }

        pos = row_start + tr_close + 5;
    }

    rows
}

fn extract_row_cells(row_html: &str) -> Vec<(bool, String)> {
    let lower = row_html.to_lowercase();
    let mut cells = Vec::new();
    let mut pos = 0;

    while pos < row_html.len() {
        let cell_start = if let Some(th) = lower[pos..].find("<th") {
            Some((pos + th, true))
        } else {
            lower[pos..].find("<td").map(|td| (pos + td, false))
        };

        let Some((abs_cell, is_th)) = cell_start else {
            break;
        };

        let Some(gt) = lower[abs_cell..].find('>') else {
            break;
        };
        let content_start = abs_cell + gt + 1;

        let close_tag = if is_th { "</th>" } else { "</td>" };
        let Some(close) = lower[content_start..].find(close_tag) else {
            break;
        };

        let inner = &row_html[content_start..content_start + close];
        let text = strip_all_tags(inner).trim().to_string();
        cells.push((is_th, text));

        pos = content_start + close + close_tag.len();
    }

    cells
}

// ---------------------------------------------------------------------------
// HTML string-scanning helpers (no external regex crate needed)
// ---------------------------------------------------------------------------

fn extract_title_tag(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let open = lower.find("<title")?;
    let gt = lower[open..].find('>')? + open + 1;
    let close = lower.find("</title>")?;
    if close <= gt {
        return None;
    }
    Some(html[gt..close].trim().to_string())
}

fn extract_meta_by_name(html: &str, name: &str, entries: &mut Vec<(String, String)>) {
    let lower = html.to_lowercase();
    let needle_name = format!("name=\"{}\"", name.to_lowercase());
    let needle_name_s = format!("name='{}'", name.to_lowercase());

    let mut pos = 0;
    while pos < html.len() {
        let Some(meta_start) = lower[pos..]
            .find("<meta ")
            .or_else(|| lower[pos..].find("<meta\t"))
        else {
            break;
        };
        let abs_start = pos + meta_start;
        let Some(end) = lower[abs_start..]
            .find('>')
            .or_else(|| lower[abs_start..].find("/>"))
        else {
            break;
        };
        let tag = &lower[abs_start..abs_start + end];
        let tag_orig = &html[abs_start..abs_start + end];

        if tag.contains(&needle_name) || tag.contains(&needle_name_s) {
            if let Some(val) = extract_attr_value(tag_orig, "content") {
                entries.push((name.to_string(), val));
                return;
            }
        }
        pos = abs_start + end + 1;
    }
}

fn extract_og_tags(html: &str, entries: &mut Vec<(String, String)>) {
    let lower = html.to_lowercase();
    let mut pos = 0;
    let mut seen = std::collections::HashSet::new();

    while pos < html.len() {
        let Some(meta_start) = lower[pos..]
            .find("<meta ")
            .or_else(|| lower[pos..].find("<meta\t"))
        else {
            break;
        };
        let abs_start = pos + meta_start;
        let Some(end) = lower[abs_start..]
            .find('>')
            .or_else(|| lower[abs_start..].find("/>"))
        else {
            break;
        };
        let tag = &lower[abs_start..abs_start + end];
        let tag_orig = &html[abs_start..abs_start + end];

        let property = extract_attr_value(tag_orig, "property")
            .or_else(|| {
                extract_attr_value(tag, "property")
                    .map(|_| extract_attr_value(tag_orig, "property").unwrap_or_default())
            })
            .unwrap_or_default();

        if property.starts_with("og:") && !seen.contains(&property) {
            if let Some(val) = extract_attr_value(tag_orig, "content") {
                seen.insert(property.clone());
                entries.push((property, val));
            }
        }
        pos = abs_start + end + 1;
    }
}

fn extract_canonical(html: &str, entries: &mut Vec<(String, String)>) {
    let lower = html.to_lowercase();
    let mut pos = 0;
    while pos < html.len() {
        let Some(link_start) = lower[pos..].find("<link ") else {
            break;
        };
        let abs_start = pos + link_start;
        let Some(end) = lower[abs_start..]
            .find('>')
            .or_else(|| lower[abs_start..].find("/>"))
        else {
            break;
        };
        let tag = &lower[abs_start..abs_start + end];
        let tag_orig = &html[abs_start..abs_start + end];

        if tag.contains("rel=\"canonical\"") || tag.contains("rel='canonical'") {
            if let Some(val) = extract_attr_value(tag_orig, "href") {
                entries.push(("canonical".to_string(), val));
                return;
            }
        }
        pos = abs_start + end + 1;
    }
}

fn extract_time_tags(html: &str, entries: &mut Vec<(String, String)>) {
    let lower = html.to_lowercase();
    let mut pos = 0;
    while pos < html.len() {
        let Some(open) = lower[pos..].find("<time") else {
            break;
        };
        let abs_open = pos + open;
        let Some(gt) = lower[abs_open..].find('>') else {
            break;
        };
        let content_start = abs_open + gt + 1;
        let Some(close) = lower[content_start..].find("</time>") else {
            break;
        };
        let inner = &html[content_start..content_start + close];
        let text = strip_all_tags(inner).trim().to_string();
        if !text.is_empty() {
            entries.push(("time".to_string(), text));
        }
        pos = content_start + close + 7;
    }
}

fn extract_anchor_tags(html: &str) -> Vec<(String, String)> {
    let lower = html.to_lowercase();
    let mut results = Vec::new();
    let mut pos = 0;

    while pos < html.len() {
        let Some(a_start) = lower[pos..]
            .find("<a ")
            .or_else(|| lower[pos..].find("<a\t"))
        else {
            break;
        };
        let abs_start = pos + a_start;
        let Some(gt) = lower[abs_start..].find('>') else {
            break;
        };
        let tag_attrs = &html[abs_start..abs_start + gt];
        let content_start = abs_start + gt + 1;

        let Some(close) = lower[content_start..].find("</a>") else {
            break;
        };
        let inner = &html[content_start..content_start + close];

        if let Some(href) = extract_attr_value(tag_attrs, "href") {
            if href.is_empty() || href == "#" {
                pos = content_start + close + 4;
                continue;
            }
            let text = strip_all_tags(inner).trim().to_string();
            let text = if text.is_empty() { href.clone() } else { text };
            results.push((text, href));
        }
        pos = content_start + close + 4;
    }
    results
}

fn extract_attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower_tag = tag.to_lowercase();
    let needle_dq = format!("{}=\"", attr);
    let needle_sq = format!("{}='", attr);

    // Check double-quoted.
    if let Some(idx) = lower_tag.find(&needle_dq) {
        let val_start = idx + needle_dq.len();
        if let Some(end) = tag[val_start..].find('"') {
            return Some(tag[val_start..val_start + end].to_string());
        }
    }

    // Check single-quoted.
    if let Some(idx) = lower_tag.find(&needle_sq) {
        let val_start = idx + needle_sq.len();
        if let Some(end) = tag[val_start..].find('\'') {
            return Some(tag[val_start..val_start + end].to_string());
        }
    }

    None
}

fn strip_element_contents(html: &str, tags: &[&str]) -> String {
    let lower = html.to_lowercase();
    let mut result = String::with_capacity(html.len());
    let mut pos = 0;

    'outer: while pos < html.len() {
        if html.as_bytes()[pos] == b'<' {
            let rest_lower = &lower[pos..];
            for &tag in tags {
                let open_needle = format!("<{tag}");
                if rest_lower.starts_with(&open_needle) {
                    let close_needle = format!("</{tag}>");
                    if let Some(end_idx) = rest_lower.find(&close_needle) {
                        pos += end_idx + close_needle.len();
                        continue 'outer;
                    }
                    // Self-closing or unclosed — skip just this tag.
                    if let Some(gt) = rest_lower.find('>') {
                        pos += gt + 1;
                        continue 'outer;
                    }
                }
            }
        }
        result.push(html.as_bytes()[pos] as char);
        pos += 1;
    }
    result
}

fn strip_html_comments(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut pos = 0;
    while pos < html.len() {
        if html[pos..].starts_with("<!--") {
            if let Some(end) = html[pos + 4..].find("-->") {
                pos += end + 7;
            } else {
                break;
            }
        } else {
            result.push(html.as_bytes()[pos] as char);
            pos += 1;
        }
    }
    result
}

fn strip_all_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

fn collapse_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_was_ws {
                result.push(' ');
                last_was_ws = true;
            }
        } else {
            result.push(ch);
            last_was_ws = false;
        }
    }
    result.trim().to_string()
}
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

    // ── extract_with_mode tests ──────────────────────────────────────

    static FIXTURE_HTML: &str = r#"<html>
<head>
    <title>Test Page</title>
    <meta name="description" content="A test page for extraction">
    <meta name="author" content="Alice">
    <meta property="og:title" content="OG Test Page">
    <meta property="og:description" content="OG desc">
    <link rel="canonical" href="https://example.com/canonical">
    <style>body { color: red; }</style>
</head>
<body>
    <header>Site Header</header>
    <nav><a href="/">Home</a></nav>
    <main>
        <article>
            <h1>Hello World</h1>
            <p>This is the <strong>main content</strong>.</p>
            <p>Read more <a href="https://example.com/docs">documentation</a>.</p>
            <p>Also see <a href="https://rust-lang.org">Rust</a>.</p>
            <time datetime="2025-01-15">January 15, 2025</time>
        </article>
    </main>
    <footer>Footer text</footer>
    <script>alert("hi");</script>
</body>
</html>"#;

    #[test]
    fn test_extract_with_mode_markdown_delegates_to_extract() {
        let result = extract_with_mode(
            "<html><body><article><h1>Hi</h1><p>Content</p></article></body></html>",
            "https://example.com/",
            "text/html",
            &ExtractionMode::Markdown,
        )
        .expect("should not error")
        .expect("should return content");

        assert!(result.text_content.contains("Content"));
    }

    #[test]
    fn test_extract_with_mode_text_strips_tags() {
        let result = extract_with_mode(
            FIXTURE_HTML,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Text,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.title, "Test Page");
        assert!(
            !result.text_content.contains('<'),
            "text mode should have no HTML tags: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("Hello World"),
            "should contain main heading"
        );
        assert!(
            result.text_content.contains("main content"),
            "should contain body text"
        );
        assert!(
            !result.text_content.contains("alert"),
            "script content should be stripped"
        );
        assert!(
            !result.text_content.contains("color: red"),
            "style content should be stripped"
        );
    }

    #[test]
    fn test_extract_with_mode_html_removes_boilerplate() {
        let result = extract_with_mode(
            FIXTURE_HTML,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Html,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.title, "Test Page");
        assert!(
            !result.text_content.contains("<nav"),
            "nav tags should be removed"
        );
        assert!(
            !result.text_content.contains("<footer"),
            "footer tags should be removed"
        );
        assert!(
            !result.text_content.contains("<script"),
            "script tags should be removed"
        );
        assert!(
            !result.text_content.contains("<style"),
            "style tags should be removed"
        );
        assert!(
            !result.text_content.contains("<header"),
            "header tags should be removed"
        );
        assert!(
            result.text_content.contains("<article>"),
            "article tags should be preserved"
        );
        assert!(
            result.text_content.contains("<p>"),
            "p tags should be preserved"
        );
    }

    #[test]
    fn test_extract_with_mode_links_extracts_anchors() {
        let result = extract_with_mode(
            FIXTURE_HTML,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Links,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.title, "Test Page");
        assert!(
            result
                .text_content
                .contains("[documentation](https://example.com/docs)"),
            "should list docs link: got {:?}",
            result.text_content
        );
        assert!(
            result
                .text_content
                .contains("[Rust](https://rust-lang.org)"),
            "should list Rust link: got {:?}",
            result.text_content
        );
    }

    #[test]
    fn test_extract_with_mode_links_no_links_found() {
        let html = "<html><body><p>No links here</p></body></html>";
        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Links,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.text_content, "No links found on this page.");
    }

    #[test]
    fn test_extract_with_mode_metadata_extracts_all_fields() {
        let result = extract_with_mode(
            FIXTURE_HTML,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Metadata,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.title, "Test Page");
        assert!(
            result.text_content.contains("title: Test Page"),
            "should have title: got {:?}",
            result.text_content
        );
        assert!(
            result
                .text_content
                .contains("description: A test page for extraction"),
            "should have description"
        );
        assert!(
            result.text_content.contains("author: Alice"),
            "should have author"
        );
        assert!(
            result.text_content.contains("og:title: OG Test Page"),
            "should have og:title"
        );
        assert!(
            result.text_content.contains("og:description: OG desc"),
            "should have og:description"
        );
        assert!(
            result
                .text_content
                .contains("canonical: https://example.com/canonical"),
            "should have canonical"
        );
        assert!(
            result.text_content.contains("time:"),
            "should have time tag"
        );
    }

    #[test]
    fn test_extract_with_mode_metadata_no_metadata() {
        let html = "<html><body><p>Just text</p></body></html>";
        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Metadata,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.text_content, "No metadata found on this page.");
    }

    #[test]
    fn test_extract_with_mode_text_returns_none_for_empty() {
        let html = "<html><head><style>x</style></head><body><script>y</script></body></html>";
        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Text,
        )
        .expect("should not error");
        assert!(result.is_none(), "script/style only should return None");
    }

    // ── JsonLd mode tests ─────────────────────────────────────────────

    #[test]
    fn test_extract_with_mode_jsonld_extracts_structured_data() {
        let html = r#"<html><head>
            <title>Recipe Page</title>
            <script type="application/ld+json">{"@type":"Recipe","name":"Pancakes","ingredients":["flour","eggs","milk"]}</script>
        </head><body><p>A great recipe</p></body></html>"#;

        let result = extract_with_mode(
            html,
            "https://example.com/recipe",
            "text/html",
            &ExtractionMode::JsonLd,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.title, "Recipe Page");
        assert!(
            result.text_content.contains("\"@type\""),
            "should contain pretty-printed JSON key: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("\"Recipe\""),
            "should contain Recipe value: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("\"name\""),
            "should contain name key: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("\"Pancakes\""),
            "should contain Pancakes value: got {:?}",
            result.text_content
        );
    }

    #[test]
    fn test_extract_with_mode_jsonld_no_jsonld_found() {
        let html = "<html><head><title>Simple</title></head><body><p>No structured data here</p></body></html>";
        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::JsonLd,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(
            result.text_content,
            "No structured data (JSON-LD) found on this page."
        );
    }

    #[test]
    fn test_extract_with_mode_jsonld_multiple_blocks() {
        let html = r#"<html><head>
            <title>Multi</title>
            <script type="application/ld+json">{"@type":"Organization","name":"Acme"}</script>
            <script type="application/ld+json">{"@type":"WebPage","name":"Home"}</script>
        </head><body></body></html>"#;

        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::JsonLd,
        )
        .expect("should not error")
        .expect("should return content");

        assert!(
            result.text_content.contains("### Structured Data Block 1"),
            "should have block 1 header: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("### Structured Data Block 2"),
            "should have block 2 header: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("\"Organization\""),
            "should contain Organization: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("\"WebPage\""),
            "should contain WebPage: got {:?}",
            result.text_content
        );
    }

    // ── Tables mode tests ─────────────────────────────────────────────

    #[test]
    fn test_extract_with_mode_tables_extracts_table() {
        let html = r#"<html><head><title>Data</title></head><body>
            <table>
                <tr><th>Name</th><th>Age</th></tr>
                <tr><td>Alice</td><td>30</td></tr>
                <tr><td>Bob</td><td>25</td></tr>
            </table>
        </body></html>"#;

        let result = extract_with_mode(
            html,
            "https://example.com/data",
            "text/html",
            &ExtractionMode::Tables,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.title, "Data");
        assert!(
            result.text_content.contains("| Name | Age |"),
            "should have header row: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("| --- | --- |"),
            "should have separator: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("| Alice | 30 |"),
            "should have Alice row: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("| Bob | 25 |"),
            "should have Bob row: got {:?}",
            result.text_content
        );
    }

    #[test]
    fn test_extract_with_mode_tables_no_tables_found() {
        let html = "<html><body><p>Just a paragraph</p></body></html>";
        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Tables,
        )
        .expect("should not error")
        .expect("should return content");

        assert_eq!(result.text_content, "No tables found on this page.");
    }

    #[test]
    fn test_extract_with_mode_tables_without_th_uses_first_row() {
        let html = r#"<html><body>
            <table>
                <tr><td>Col1</td><td>Col2</td></tr>
                <tr><td>A</td><td>B</td></tr>
            </table>
        </body></html>"#;

        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Tables,
        )
        .expect("should not error")
        .expect("should return content");

        assert!(
            result.text_content.contains("| Col1 | Col2 |"),
            "first row should become header: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("| A | B |"),
            "should have data row: got {:?}",
            result.text_content
        );
    }

    #[test]
    fn test_extract_with_mode_tables_multiple_tables() {
        let html = r#"<html><body>
            <table><tr><th>X</th></tr><tr><td>1</td></tr></table>
            <table><tr><th>Y</th></tr><tr><td>2</td></tr></table>
        </body></html>"#;

        let result = extract_with_mode(
            html,
            "https://example.com/",
            "text/html",
            &ExtractionMode::Tables,
        )
        .expect("should not error")
        .expect("should return content");

        assert!(
            result.text_content.contains("### Table 1"),
            "should have Table 1 header: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("### Table 2"),
            "should have Table 2 header: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("| X |"),
            "should have X header: got {:?}",
            result.text_content
        );
        assert!(
            result.text_content.contains("| Y |"),
            "should have Y header: got {:?}",
            result.text_content
        );
    }
}
