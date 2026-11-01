//! Discovery of container image references in GitLab CI files.
//!
//! Task 2: scalar `image:` / `services:` lines only. Empty-value lines
//! produce no targets; flow (`[...]`) and block sequences are Task 3.

use crate::image_ref::ImageRef;

/// One discovered image reference location in a CI file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// 0-based line index in the file text.
    pub line: usize,
    /// Byte offset where the value token begins (incl. quotes).
    pub start: usize,
    /// Byte offset where the value token ends (excl. comments).
    pub end: usize,
    /// The value with quotes stripped.
    pub value: String,
}

/// Discover scalar `image:` / `services:` references in a CI file.
///
/// Returns targets sorted by `(line, start)`. A line is a candidate when it
/// is a top-level `image:`/`services:` line or sits one level inside a
/// top-level block (at the block's child indentation); deeper nesting is
/// ignored. A candidate yields a target only if its scalar value parses via
/// [`ImageRef::parse`].
pub fn discover(text: &str) -> Vec<Target> {
    // Precompute each line's byte offset in `text` (line text excl. newline).
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut off = 0usize;
    for chunk in text.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        lines.push((off, line));
        off += chunk.len();
    }

    let mut targets: Vec<Target> = Vec::new();
    // Current top-level key (from an indent-0 `key:` line), whether it opens
    // a block (`key:` with no value), and the indentation of the first
    // non-empty line inside that block.
    let mut top: Option<&str> = None;
    let mut in_block = false;
    let mut child_indent: Option<usize> = None;

    for (line_idx, (line_off, line)) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        // Indentation = leading spaces. A line whose leading whitespace
        // contains a tab is ignored entirely (yields no targets).
        let indent = line.len() - line.trim_start_matches(' ').len();
        let ws_end = line.len() - line.trim_start_matches([' ', '\t']).len();
        let tab_indented = ws_end != indent;
        if tab_indented {
            continue;
        }
        let content = &line[ws_end..];
        let is_comment = content.starts_with('#');
        let colon = image_key_colon(line);

        if indent == 0 {
            if !is_comment {
                if let Some((key, has_value)) = top_key(line) {
                    top = Some(key);
                    in_block = !has_value;
                    child_indent = None;
                }
            }
            // Top-level `image:` / `services:` is a candidate.
            push_target(&mut targets, line_idx, *line_off, line, colon);
        } else if !is_comment && top.is_some() && in_block {
            // One level inside a top-level block: only the block's child
            // indentation is a candidate; deeper nesting is not.
            if child_indent.is_none() {
                child_indent = Some(indent);
            }
            if Some(indent) == child_indent {
                push_target(&mut targets, line_idx, *line_off, line, colon);
            }
        }
    }

    targets.sort_by_key(|t| (t.line, t.start));
    targets
}

/// If the line is `image:` or `services:` followed by end-of-line or
/// whitespace (after leading spaces), return the byte position of the colon
/// in the line.
fn image_key_colon(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let content = &line[indent..];
    for key in ["image", "services"] {
        if let Some(rest) = content.strip_prefix(key).and_then(|r| r.strip_prefix(':')) {
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
                return Some(indent + key.len());
            }
        }
    }
    None
}

/// Parse an indent-0 `key:` or `key: value` line.
///
/// Returns the key and whether it carries a value (a `#` comment after the
/// colon does not count as a value).
fn top_key(line: &str) -> Option<(&str, bool)> {
    let colon = line.find(':')?;
    let key = &line[..colon];
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return None;
    }
    let after = &line[colon + 1..];
    if !after.is_empty() && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let value_part = after.trim_start();
    let has_value = !value_part.is_empty() && !value_part.starts_with('#');
    Some((key, has_value))
}

/// If `line` is a candidate `image:`/`services:` line (colon = byte position
/// of its colon) whose scalar value parses as an [`ImageRef`], push a target
/// on `targets`. Offsets are relative to the line; `line_off` is the line's
/// byte offset in the file text.
fn push_target(targets: &mut Vec<Target>, line_idx: usize, line_off: usize, line: &str, colon: Option<usize>) {
    let Some(colon) = colon else {
        return;
    };
    let Some((start, end, value)) = scalar_value_token(line, colon) else {
        return;
    };
    if ImageRef::parse(&value).is_ok() {
        targets.push(Target {
            line: line_idx,
            start: line_off + start,
            end: line_off + end,
            value,
        });
    }
}

/// Extract the scalar value token of a `key:` line.
///
/// Returns `(start, end, value)` with byte offsets relative to the line
/// start: the span covers the exact written token (quotes included in the
/// span, not in the value); `end` excludes any inline comment. Returns
/// `None` when the line yields no target: empty value, unclosed quote, or a
/// value that starts with `[` (flow, Task 3), `|`/`>` (block scalar) or `#`
/// (inline comment).
fn scalar_value_token(line: &str, colon: usize) -> Option<(usize, usize, String)> {
    let after = &line[colon + 1..];
    let pad = after.len() - after.trim_start().len();
    let content = &after[pad..];
    if content.is_empty() || content.starts_with('#') {
        return None;
    }
    let start = colon + 1 + pad;
    match content.chars().next()? {
        q if q == '"' || q == '\'' => {
            let close = content[1..].find(q)?;
            Some((start, start + 2 + close, content[1..1 + close].to_string()))
        }
        '[' | '|' | '>' => None,
        _ => {
            let cut = content.find(" #").unwrap_or(content.len());
            let token = content[..cut].trim_end();
            Some((start, start + token.len(), token.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = include_str!("../tests/fixtures/ci-simple.yml");

    #[test]
    fn discovers_scalar_image_and_services() {
        let t = discover(SIMPLE);
        let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line, x.value.as_str())).collect();
        assert_eq!(got, vec![
            (1, "alpine:3.20"),    // top-level image
            (4, "ubuntu:24.04"),   // default.image
            (7, "node:20"),        // build.image (quoted; value unquoted)
            (8, "redis:7"),        // build.services scalar
            (11, "alpine:3.20"),   // test.image (same image twice)
        ]);
        // spans must cover the exact written token (quotes included in span, not value)
        for x in &t {
            let token = &SIMPLE[x.start..x.end];
            let unquoted = token.trim_matches(|c| c == '"' || c == '\'');
            assert_eq!(unquoted, x.value);
        }
    }
}
