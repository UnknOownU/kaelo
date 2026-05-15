//! Post-process Markdown output from dom_smoothie to fix common issues.

/// Characters that should NOT be unescaped because they're meaningful in Markdown.
const MD_SIGNIFICANT: &[char] = &['*', '_', '#', '[', ']', '\\', '`', '~'];

/// Fix excessive backslash escaping from dom_smoothie.
///
/// Removes unnecessary `\` before common punctuation (`.`, `!`, `(`, `)`, `,`, `:`, `;`, `@`, `+`, `-`, `=`, `{`, `}`, `<`, `>`, `/`, `?`, `|`, `^`, `$`, `%`, `&`, `'`, `"`).
/// Preserves backslashes before Markdown-significant characters.
fn fix_excessive_backslashes(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                if !MD_SIGNIFICANT.contains(&next) && !next.is_whitespace() {
                    // Skip the backslash, keep the next char
                    result.push(chars.next().unwrap());
                } else {
                    // Keep the backslash (meaningful escape or trailing backslash)
                    result.push(ch);
                }
            } else {
                // Trailing backslash, keep it
                result.push(ch);
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Ensure `#` headings have a space after `#`.
///
/// `"##Title"` → `"## Title"`
/// `"###Sub"` → `"### Sub"`
fn normalize_headings(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut at_line_start = true;

    while let Some(ch) = chars.next() {
        if at_line_start {
            if ch == '\n' {
                result.push(ch);
            } else {
                at_line_start = false;
                result.push(ch);
                if ch == '#' {
                    let mut hash_count = 1usize;
                    while chars.peek() == Some(&'#') {
                        hash_count += 1;
                        result.push(chars.next().unwrap());
                    }
                    if (1..=6).contains(&hash_count) {
                        if let Some(&next) = chars.peek() {
                            if next != ' ' && next != '\n' {
                                result.push(' ');
                            }
                        }
                    }
                }
            }
        } else {
            if ch == '\n' {
                at_line_start = true;
            }
            result.push(ch);
        }
    }

    result
}

/// Remove empty links: `[text]()` → `text`.
fn remove_empty_links(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(end_bracket) = bytes[i + 1..].iter().position(|&b| b == b']') {
                let bracket_end = i + 1 + end_bracket;
                if bracket_end + 2 <= bytes.len()
                    && bytes[bracket_end + 1] == b'('
                    && bytes[bracket_end + 2] == b')'
                {
                    let text = &input[i + 1..bracket_end];
                    result.push_str(text);
                    i = bracket_end + 3;
                    continue;
                }
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }

    result
}

/// Collapse 3+ consecutive blank lines to 2.
fn collapse_blank_lines(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut blank_count = 0u32;

    for line in input.lines() {
        if line.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            result.push_str(line);
            result.push('\n');
            blank_count = 0;
        }
    }

    result
}

/// Post-process Markdown output from dom_smoothie to fix common issues.
pub fn clean_markdown(input: &str) -> String {
    let mut output = input.to_string();
    output = fix_excessive_backslashes(&output);
    output = normalize_headings(&output);
    output = remove_empty_links(&output);
    output = collapse_blank_lines(&output);
    output.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_excessive_backslashes() {
        assert_eq!(
            fix_excessive_backslashes(r"features\. here"),
            "features. here"
        );
        assert_eq!(fix_excessive_backslashes(r"wow\! indeed"), "wow! indeed");
        assert_eq!(fix_excessive_backslashes(r"see \(note\)"), "see (note)");
        assert_eq!(fix_excessive_backslashes(r"items\, list\:"), "items, list:");
        // Markdown-significant chars are preserved
        assert_eq!(fix_excessive_backslashes(r"\*italic\*"), r"\*italic\*");
        assert_eq!(
            fix_excessive_backslashes(r"\_underscore\_"),
            r"\_underscore\_"
        );
        assert_eq!(fix_excessive_backslashes(r"\# heading"), r"\# heading");
        assert_eq!(fix_excessive_backslashes(r"\[link\]"), r"\[link\]");
        // Backslash before whitespace is kept
        assert_eq!(fix_excessive_backslashes(r"hello\ world"), r"hello\ world");
        // No backslashes = no change
        assert_eq!(fix_excessive_backslashes("plain text"), "plain text");
    }

    #[test]
    fn test_normalize_headings() {
        assert_eq!(normalize_headings("##No space"), "## No space");
        assert_eq!(normalize_headings("###Sub heading"), "### Sub heading");
        assert_eq!(normalize_headings("#Title"), "# Title");
        assert_eq!(normalize_headings("## Already spaced"), "## Already spaced");
        // Headings already with newline-space are unchanged
        assert_eq!(normalize_headings("## spaced"), "## spaced");
    }

    #[test]
    fn test_remove_empty_links() {
        assert_eq!(remove_empty_links("[click]()"), "click");
        assert_eq!(
            remove_empty_links("see [here]() for more"),
            "see here for more"
        );
        // Non-empty links are preserved
        assert_eq!(
            remove_empty_links("[link](https://example.com)"),
            "[link](https://example.com)"
        );
        // No links = no change
        assert_eq!(remove_empty_links("plain text"), "plain text");
    }

    #[test]
    fn test_collapse_blank_lines() {
        // 4 blank lines → 2
        assert_eq!(
            collapse_blank_lines("hello\n\n\n\n\nworld"),
            "hello\n\n\nworld\n"
        );
        // 2 blank lines → unchanged (2 blank lines means 3 newlines total → 2 blank lines)
        assert_eq!(
            collapse_blank_lines("hello\n\n\nworld"),
            "hello\n\n\nworld\n"
        );
        // 1 blank line → unchanged
        assert_eq!(collapse_blank_lines("hello\n\nworld"), "hello\n\nworld\n");
        // No blank lines
        assert_eq!(collapse_blank_lines("hello\nworld"), "hello\nworld\n");
    }

    #[test]
    fn test_clean_markdown_combined() {
        let messy = r#"#Article Title

Some intro text\.

##Subheading

See [this]() for info\, and [real](https://example.com) too.



More text\!




Even more"#;

        let cleaned = clean_markdown(messy);

        let expected = "# Article Title\n\nSome intro text.\n\n## Subheading\n\nSee this for info, and [real](https://example.com) too.\n\n\nMore text!\n\n\nEven more";
        assert_eq!(
            cleaned.as_bytes(),
            expected.as_bytes(),
            "cleaned={:?} expected={:?}",
            cleaned,
            expected
        );
    }
}
