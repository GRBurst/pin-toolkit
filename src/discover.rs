//! Discovery of container image references in GitLab CI files.
//!
//! Task 2 + 3: scalar, flow-sequence (`[...]`) and block-sequence
//! (`- ...`) `image:` / `services:` values, plus `image:` as a mapping
//! (`name:` child). Comment lines, `include:` blocks and script items
//! never produce targets.

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

/// Discover `image:` / `services:` references in a CI file.
///
/// Returns targets sorted by `(line, start)`. A line is a candidate when it
/// is a top-level `image:`/`services:` line or sits one level inside a
/// top-level block (at the block's child indentation); deeper nesting is
/// ignored. Syntactic candidates yield targets: a bare/quoted scalar (one
/// target), a flow sequence (one target per item), or an empty value opening
/// a block — sequence items (`- value`, `- name: value`) and, for `image:`,
/// a `name:` child at the block's child indentation. Comment lines never
/// produce targets.
///
/// Discovery is purely syntactic — it does NOT filter via
/// [`ImageRef::parse`](crate::image_ref::ImageRef::parse). Values that fail to parse (e.g. shell/YAML
/// interpolation like `${IMAGE}:${TAG}`) stay targets and are reported as
/// `skipped (unparseable reference)` at resolve time, so the report stays
/// complete and the exit code stays 0.
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
    // Inside the block of an empty-value `image:`/`services:` key line:
    // the key's indentation, whether the key is `image` (a `name:` child is
    // then a target), and the indentation of the first non-item line in the
    // block.
    let mut sub_key_indent: Option<usize> = None;
    let mut sub_is_image = false;
    let mut sub_child_indent: Option<usize> = None;

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
        // Comment lines never produce targets and never change state.
        if content.starts_with('#') {
            continue;
        }

        // Block of an empty-value `image:`/`services:` key (Task 3).
        if let Some(key_indent) = sub_key_indent {
            if indent > key_indent {
                if let Some(rest) = dash_item_rest(line) {
                    push_item_targets(&mut targets, line_idx, *line_off, line, rest);
                } else if sub_is_image {
                    if sub_child_indent.is_none() {
                        sub_child_indent = Some(indent);
                    }
                    if Some(indent) == sub_child_indent {
                        push_name_child_target(&mut targets, line_idx, *line_off, line, indent);
                    }
                }
                continue;
            }
            if indent == key_indent
                && let Some(rest) = dash_item_rest(line)
            {
                // A `- item` at the key's indentation belongs to the block.
                push_item_targets(&mut targets, line_idx, *line_off, line, rest);
                continue;
            }
            // Any other line at or above the key's indentation ends the block.
            sub_key_indent = None;
            sub_child_indent = None;
        }

        let colon = image_key_colon(line);

        let candidate = if indent == 0 {
            if let Some((key, has_value)) = top_key(line) {
                top = Some(key);
                in_block = !has_value;
                child_indent = None;
            }
            // Top-level `image:` / `services:` is a candidate.
            true
        } else if top.is_some() && in_block {
            // One level inside a top-level block: only the block's child
            // indentation is a candidate; deeper nesting is not.
            if child_indent.is_none() {
                child_indent = Some(indent);
            }
            Some(indent) == child_indent
        } else {
            false
        };

        if candidate {
            push_target(&mut targets, line_idx, *line_off, line, colon);
            // An empty-value `image:`/`services:` key opens a block (Task 3).
            if let Some((key, k_colon)) = image_key(line) {
                let value = line[k_colon + 1..].trim_start();
                if value.is_empty() || value.starts_with('#') {
                    sub_key_indent = Some(indent);
                    sub_is_image = key == "image";
                    sub_child_indent = None;
                }
            }
        }
    }

    targets.sort_by_key(|t| (t.line, t.start));
    targets
}

/// If the line is `image:` or `services:` followed by end-of-line or
/// whitespace (after leading spaces), return the key and the byte position
/// of the colon in the line.
fn image_key(line: &str) -> Option<(&'static str, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let content = &line[indent..];
    for key in ["image", "services"] {
        if let Some(rest) = content.strip_prefix(key).and_then(|r| r.strip_prefix(':'))
            && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'))
        {
            return Some((key, indent + key.len()));
        }
    }
    None
}

/// The byte position of the colon of an `image:`/`services:` line.
fn image_key_colon(line: &str) -> Option<usize> {
    image_key(line).map(|(_, colon)| colon)
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
/// of its colon), push its target(s) on `targets`: a scalar value, or one
/// target per item of a flow sequence. Offsets are relative to the line;
/// `line_off` is the line's byte offset in the file text.
fn push_target(
    targets: &mut Vec<Target>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    colon: Option<usize>,
) {
    let Some(colon) = colon else {
        return;
    };
    let after = &line[colon + 1..];
    let pad = after.len() - after.trim_start().len();
    let content = &after[pad..];
    if content.is_empty() || content.starts_with('#') {
        return;
    }
    let start = colon + 1 + pad;
    if content.starts_with('[') {
        push_flow_targets(targets, line_idx, line_off, line, start);
        return;
    }
    let Some((start, end, value)) = scalar_token(content, start) else {
        return;
    };
    push_if_parses(targets, line_idx, line_off, start, end, value);
}

/// Push one target per image-ref item of the flow sequence whose `[` is at
/// `bracket` in `line`. M1: the inside must not contain `{`, `}`, or a
/// nested `[` (otherwise the whole line yields no targets); items are the
/// comma-split pieces — trimmed, one layer of quotes stripped — and each
/// must parse as an [`ImageRef`](crate::image_ref::ImageRef). Spans point at the item within the line
/// (quotes included in the span when quoted).
fn push_flow_targets(
    targets: &mut Vec<Target>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    bracket: usize,
) {
    let Some(close) = line[bracket..].find(']') else {
        return; // unclosed: not a flow sequence
    };
    let inside_off = bracket + 1;
    let inside = &line[inside_off..bracket + close];
    if inside.contains(['{', '}', '[']) {
        return;
    }
    let mut part_off = 0usize;
    for part in inside.split(',') {
        let pad = part.len() - part.trim_start().len();
        let token = part.trim();
        if !token.is_empty() {
            push_flow_item(
                targets,
                line_idx,
                line_off,
                inside_off + part_off + pad,
                token,
            );
        }
        part_off += part.len() + 1; // the part plus its comma
    }
}

/// Push a target for one flow item: the trimmed `token` starting at `tok_off`
/// in the line, with one layer of quotes stripped when present (the span
/// includes the quotes).
fn push_flow_item(
    targets: &mut Vec<Target>,
    line_idx: usize,
    line_off: usize,
    tok_off: usize,
    token: &str,
) {
    let (len, value) = match token.chars().next().unwrap() {
        q if q == '"' || q == '\'' => match token[1..].find(q) {
            Some(close) => (2 + close, token[1..1 + close].to_string()),
            None => return, // unclosed quote: skip the item
        },
        _ => (token.len(), token.to_string()),
    };
    push_if_parses(targets, line_idx, line_off, tok_off, tok_off + len, value);
}

/// If `line` is a block-sequence item — leading spaces, `-`, whitespace, then
/// non-empty content — return the byte offset in the line where the item rest
/// begins.
fn dash_item_rest(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let after_dash = line[indent..].strip_prefix('-')?;
    let pad = after_dash.len() - after_dash.trim_start().len();
    if pad == 0 || after_dash[pad..].is_empty() {
        return None;
    }
    Some(indent + 1 + pad)
}

/// Push a target for a block-sequence item whose rest begins at `rest` in
/// `line`: either a `name: <scalar>` mapping (target = the scalar) or a
/// bare/quoted scalar (target = the scalar itself).
fn push_item_targets(
    targets: &mut Vec<Target>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    rest: usize,
) {
    let rest_str = &line[rest..];
    let (value_off, value_content) = match name_value_offset(rest_str) {
        Some(pos) => {
            let after = &rest_str[pos..];
            let pad = after.len() - after.trim_start().len();
            (rest + pos + pad, &after[pad..])
        }
        None => (rest, rest_str),
    };
    let Some((start, end, value)) = scalar_token(value_content, value_off) else {
        return;
    };
    push_if_parses(targets, line_idx, line_off, start, end, value);
}

/// If `content` is `name:` followed by end-of-line or whitespace, return the
/// offset of the byte just after the colon.
fn name_value_offset(content: &str) -> Option<usize> {
    let after = content.strip_prefix("name")?.strip_prefix(':')?;
    if after.is_empty() || after.starts_with(' ') || after.starts_with('\t') {
        Some(5) // just after `name:`
    } else {
        None
    }
}

/// Push a target for a `name:` child of an `image:` mapping block: the line's
/// content (starting at `indent`) must be `name: <scalar>`.
fn push_name_child_target(
    targets: &mut Vec<Target>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    indent: usize,
) {
    let content = &line[indent..];
    let Some(pos) = name_value_offset(content) else {
        return;
    };
    let after = &content[pos..];
    let pad = after.len() - after.trim_start().len();
    let Some((start, end, value)) = scalar_token(&after[pad..], indent + pos + pad) else {
        return;
    };
    push_if_parses(targets, line_idx, line_off, start, end, value);
}

/// Push a target for the parsed token `value` written at `[start, end)` in
/// the line (byte offsets relative to the line).
fn push_if_parses(
    targets: &mut Vec<Target>,
    line_idx: usize,
    line_off: usize,
    start: usize,
    end: usize,
    value: String,
) {
    targets.push(Target {
        line: line_idx,
        start: line_off + start,
        end: line_off + end,
        value,
    });
}

/// Extract the scalar token at the start of `content` (a slice of a line
/// without leading whitespace).
///
/// Returns `(start, end, value)` with byte offsets relative to the line
/// start (`base` is the line offset of `content`'s first byte): the span
/// covers the exact written token (quotes included in the span, not in the
/// value); `end` excludes any inline comment. Returns `None` when no token
/// is present: empty, unclosed quote, or a token starting with `[`, `|`,
/// `>` or `#`.
fn scalar_token(content: &str, base: usize) -> Option<(usize, usize, String)> {
    match content.chars().next()? {
        q if q == '"' || q == '\'' => {
            let close = content[1..].find(q)?;
            Some((base, base + 2 + close, content[1..1 + close].to_string()))
        }
        '[' | '|' | '>' | '#' => None,
        _ => {
            let cut = content.find(" #").unwrap_or(content.len());
            let token = content[..cut].trim_end();
            Some((base, base + token.len(), token.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = include_str!("../tests/fixtures/ci-simple.yml");

    #[test]
    fn templated_value_is_still_a_target() {
        // Shell/YAML interpolation is a syntactic image candidate; the parse
        // rejection happens at resolve time (reported as `skipped
        // (unparseable reference)`), not here — discover must not silently
        // drop lines the user should see in the report.
        let text = "job:\n  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}\n";
        let t = discover(text);
        assert_eq!(t.len(), 1, "templated value must stay a target");
        assert_eq!(t[0].value, "${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}");
    }

    #[test]
    fn discovers_scalar_image_and_services() {
        let t = discover(SIMPLE);
        let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line, x.value.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (1, "alpine:3.20"),  // top-level image
                (4, "ubuntu:24.04"), // default.image
                (7, "node:20"),      // build.image (quoted; value unquoted)
                (8, "redis:7"),      // build.services scalar
                (11, "alpine:3.20"), // test.image (same image twice)
            ]
        );
        // spans must cover the exact written token (quotes included in span, not value)
        for x in &t {
            let token = &SIMPLE[x.start..x.end];
            let unquoted = token.trim_matches(|c| c == '"' || c == '\'');
            assert_eq!(unquoted, x.value);
        }
    }

    const RICH: &str = include_str!("../tests/fixtures/ci-rich.yml");

    #[test]
    fn discovers_sequences_flow_and_mapping_forms() {
        let t = discover(RICH);
        let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line, x.value.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (2, "alpine:3.20"), // default.image with trailing comment
                (3, "docker:24"),   // flow item 1
                (3, "redis:7"),     // flow item 2 (same line, later span)
                (7, "node:20"),     // image mapping name
                (10, "docker:24"),  // sequence scalar
                (11, "redis:7"),    // sequence mapping name
            ]
        );
    }

    #[test]
    fn include_block_and_script_items_are_not_targets() {
        let t = discover(RICH);
        assert!(t.iter().all(|x| x.value != "v1.0.0"));
        assert!(t.iter().all(|x| x.value != "echo hi"));
    }

    #[test]
    fn trailing_comment_preserved_in_spans() {
        let t = discover(RICH);
        let first = &t[0];
        assert_eq!(&RICH[first.start..first.end], "alpine:3.20");
    }
}
