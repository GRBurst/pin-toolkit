//! Discovery of container image, component and classic include references
//! in GitLab CI files.
//!
//! Task 2 + 3: scalar, flow-sequence (`[...]`) and block-sequence
//! (`- ...`) `image:` / `services:` values, plus `image:` as a mapping
//! (`name:` child). Comment lines and script items never produce targets.
//!
//! M3 Task 1: top-level (indent 0) `component:` values become
//! `PinTarget::Component` targets; `parts` is `None` when the value fails
//! [`parse_component_ref`].
//!
//! M3 Task 2: top-level (indent 0) `include:` values that are
//! `project`/`ref` mappings become `PinTarget::IncludeRef` targets: flow
//! single mappings, flow sequence items, block sequence items and block
//! single mappings, in any key order. String/remote/template/local items
//! and mappings without both `project` and `ref` yield no target.

/// One discovered reference location in a CI file (M3: image, component or
/// classic include).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinTarget {
    /// A container image reference (`image:` / `services:` value).
    Image {
        /// 0-based line index in the file text.
        line: usize,
        /// Byte offset where the value token begins (incl. quotes).
        start: usize,
        /// Byte offset where the value token ends (excl. comments).
        end: usize,
        /// The value with quotes stripped.
        value: String,
    },
    /// A top-level GitLab component reference (`component:` value).
    Component {
        /// 0-based line index in the file text.
        line: usize,
        /// Byte offset where the value token begins (incl. quotes).
        start: usize,
        /// Byte offset where the value token ends (excl. comments).
        end: usize,
        /// The value with quotes stripped.
        value: String,
        /// Parsed `host / project / name @ version` pieces, or `None` when
        /// the value is unparseable (reported `skipped` at resolve time —
        /// the target is kept so the report stays complete).
        parts: Option<ComponentParts>,
    },
    /// A classic top-level (indent 0) `include:` mapping with explicit
    /// `project` and `ref` keys: the span points at the `ref` VALUE token
    /// (the value to be replaced with a commit SHA), not the mapping.
    IncludeRef {
        /// 0-based line index of the `ref` value token in the file text.
        line: usize,
        /// Byte offset where the `ref` value token begins (incl. quotes).
        start: usize,
        /// Byte offset where the `ref` value token ends (excl. comments).
        end: usize,
        /// The `ref` value, quotes stripped.
        value: String,
        /// The `project` path, quotes stripped.
        project: String,
        /// True when the mapping is a flow mapping
        /// (`{ project: ..., ref: ... }`).
        flow: bool,
    },
}

/// Parsed pieces of a component reference
/// `host/group[/group…]/name@version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentParts {
    /// The GitLab instance host (e.g. `gitlab.com`).
    pub host: String,
    /// The project path (e.g. `group/sub/project`).
    pub project: String,
    /// The component name (last path segment).
    pub name: String,
    /// The version token after the last `@`.
    pub version: String,
}

impl PinTarget {
    /// 0-based line index in the file text.
    pub fn line(&self) -> usize {
        match self {
            Self::Image { line, .. }
            | Self::Component { line, .. }
            | Self::IncludeRef { line, .. } => *line,
        }
    }

    /// Byte offset where the value token begins (incl. quotes).
    pub fn start(&self) -> usize {
        match self {
            Self::Image { start, .. }
            | Self::Component { start, .. }
            | Self::IncludeRef { start, .. } => *start,
        }
    }

    /// Byte offset where the value token ends (excl. comments).
    pub fn end(&self) -> usize {
        match self {
            Self::Image { end, .. }
            | Self::Component { end, .. }
            | Self::IncludeRef { end, .. } => *end,
        }
    }

    /// The value with quotes stripped.
    ///
    /// For `IncludeRef` this is the `ref` value (not `project`).
    pub fn value(&self) -> &str {
        match self {
            Self::Image { value, .. }
            | Self::Component { value, .. }
            | Self::IncludeRef { value, .. } => value,
        }
    }
}

/// Discover `image:` / `services:` / `component:` / `include:` references
/// in a CI file.
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
/// Top-level (indent 0) `component:` lines also yield targets: the value is
/// a scalar token (quotes stripped from `value`, included in the span); an
/// empty or comment-only value yields no target. `parse_component_ref`
/// splits the value at its last `@` into
/// `host / group(s) / name @ version`; `parts` is `Some` only when the
/// prefix has ≥ 3 non-empty '/'-separated segments, the host contains a `.`
/// and no `$`, and the version is non-empty.
///
/// Discovery is purely syntactic — it does NOT filter via
/// [`ImageRef::parse`](crate::image_ref::ImageRef::parse). Values that fail to parse (e.g. shell/YAML
/// interpolation like `${IMAGE}:${TAG}`) or fail
/// [`parse_component_ref`] stay targets and are reported as
/// `skipped (unparseable reference)` at resolve time, so the report stays
/// complete and the exit code stays 0.
///
/// Top-level (indent 0) `include:` lines whose value is a `project`/`ref`
/// mapping (flow single mapping, flow sequence item, block sequence item,
/// block single mapping — any key order) also yield targets: the span is
/// the `ref` VALUE token (quotes in the span, stripped in the value).
/// String, remote, template and local items — and mappings lacking either
/// `project` or `ref` — yield no target.
pub fn discover(text: &str) -> Vec<PinTarget> {
    // Precompute each line's byte offset in `text` (line text excl. newline).
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut off = 0usize;
    for chunk in text.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        lines.push((off, line));
        off += chunk.len();
    }

    let mut targets: Vec<PinTarget> = Vec::new();
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
    // Block of an indent-0 empty-value `include:` key (M3 Task 2), if
    // open.
    let mut inc_block: Option<IncludeBlock> = None;

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

        // Block of an indent-0 empty-value `include:` key (M3 Task 2):
        // inner lines are consumed by the include scanner; a non-dash line
        // at column 0 ends the block and is processed normally below.
        if inc_block.is_some() {
            if indent > 0 || dash_item_rest(line).is_some() {
                if let Some(block) = inc_block.as_mut() {
                    handle_include_inner(&mut targets, line_idx, *line_off, line, indent, block);
                }
                continue;
            }
            inc_block = None;
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
            push_component_target(&mut targets, line_idx, *line_off, line);
            push_include_target(&mut targets, line_idx, *line_off, line, &mut inc_block);
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

    targets.sort_by_key(|t| (t.line(), t.start()));
    targets
}

/// The byte position of the colon of a top-level (indent 0) `component:`
/// line, or `None`. The key must be followed by end-of-line or whitespace.
fn component_key(line: &str) -> Option<usize> {
    if let Some(rest) = line.strip_prefix("component:")
        && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'))
    {
        Some(9) // just after `component:`
    } else {
        None
    }
}

/// If `line` is a top-level `component:` line, push a
/// `PinTarget::Component` for its scalar value (quotes stripped from the
/// value, included in the span); an empty or comment-only value yields no
/// target. `parts` is `None` when the value is unparseable — the target is
/// kept so the report stays complete.
fn push_component_target(
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    line: &str,
) {
    let Some(colon) = component_key(line) else {
        return;
    };
    let after = &line[colon + 1..];
    let pad = after.len() - after.trim_start().len();
    let content = &after[pad..];
    if content.is_empty() || content.starts_with('#') {
        return;
    }
    let Some((start, end, value)) = scalar_token(content, colon + 1 + pad) else {
        return;
    };
    targets.push(PinTarget::Component {
        line: line_idx,
        start: line_off + start,
        end: line_off + end,
        parts: parse_component_ref(&value),
        value,
    });
}

/// Split a component reference `host/group[/group…]/name@version` into
/// pieces.
///
/// The split is at the LAST `@`. The prefix must have at least 3 non-empty
/// '/'-separated segments; the first (host) must contain a `.` and no `$`;
/// the version (after the last `@`) must be non-empty. Anything else is
/// `None` (the caller keeps the target and reports it `skipped` at resolve
/// time).
fn parse_component_ref(value: &str) -> Option<ComponentParts> {
    let (prefix, version) = value.rsplit_once('@')?;
    if version.is_empty() {
        return None;
    }
    let segments: Vec<&str> = prefix.split('/').collect();
    if segments.len() < 3 || segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    let host = segments[0];
    if !host.contains('.') || host.contains('$') {
        return None;
    }
    Some(ComponentParts {
        host: host.to_string(),
        project: segments[1..segments.len() - 1].join("/"),
        name: segments[segments.len() - 1].to_string(),
        version: version.to_string(),
    })
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
    targets: &mut Vec<PinTarget>,
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
    targets: &mut Vec<PinTarget>,
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
    targets: &mut Vec<PinTarget>,
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
    targets: &mut Vec<PinTarget>,
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
    targets: &mut Vec<PinTarget>,
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
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    start: usize,
    end: usize,
    value: String,
) {
    targets.push(PinTarget::Image {
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

// ── M3 Task 2: classic include: discovery ─────────────────────────────

/// State of an open indent-0 empty-value `include:` block.
#[derive(Debug)]
struct IncludeBlock {
    /// `Some(true)` = block sequence, `Some(false)` = single mapping,
    /// `None` = first inner line not seen yet.
    seq: Option<bool>,
    /// Indent of the first inner line (the block's child indent).
    child_indent: Option<usize>,
    /// Key indent of the current `- key: val` sequence item, if any.
    item_key_indent: Option<usize>,
    /// Keys collected for the current mapping (single-mapping form or
    /// current block-mapping item), if any.
    pending: Option<PendingInclude>,
}

/// A partially collected include mapping: it becomes a target the moment
/// both `project` and `ref` scalar values are present (any key order).
#[derive(Debug)]
struct PendingInclude {
    project: Option<String>,
    /// `(line, start, end, value)` of the `ref` value token; byte offsets
    /// are absolute in the file text.
    ref_tok: Option<(usize, usize, usize, String)>,
    /// True for flow-mapping items.
    flow: bool,
}

/// When `line` is a top-level (indent 0) `include:` line — `include:`
/// followed by end-of-line or whitespace — return the byte position of its
/// colon. Indented lines (job-internal includes) are out of scope.
fn include_key(line: &str) -> Option<usize> {
    if let Some(rest) = line.strip_prefix("include:")
        && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'))
    {
        Some(7) // byte position of the colon
    } else {
        None
    }
}

/// Handle a top-level `include:` line (M3 Task 2). A flow value (`{...}`
/// or `[...]`) is scanned in place; an empty (or comment-only) value opens
/// a block tracked by `discover`. Scalar includes (plain or quoted
/// strings, remote URLs) are not targets.
fn push_include_target(
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    block: &mut Option<IncludeBlock>,
) {
    let Some(colon) = include_key(line) else {
        return;
    };
    // A new top-level `include:` line never continues an older block (the
    // loop closes that on dedent), so start from a clean state.
    *block = None;
    let after = &line[colon + 1..];
    let pad = after.len() - after.trim_start().len();
    let content = &after[pad..];
    if content.is_empty() || content.starts_with('#') {
        *block = Some(IncludeBlock {
            seq: None,
            child_indent: None,
            item_key_indent: None,
            pending: None,
        });
        return;
    }
    match content.chars().next() {
        Some('{') => push_include_flow_mapping(targets, line_idx, line_off, line, colon + 1 + pad),
        Some('[') => push_include_flow_sequence(targets, line_idx, line_off, line, colon + 1 + pad),
        _ => {}
    }
}

/// The offset of the closer matching the flow opener at `open` (`{` or
/// `[`), skipping quoted content and tracking brace/bracket depth; `None`
/// when the collection is not closed on the line.
fn flow_close(line: &str, open: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    for (i, ch) in line[open + 1..].char_indices() {
        match quote {
            Some(q) if ch == q => quote = None,
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '{' || ch == '[' => depth += 1,
            None if ch == '}' || ch == ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + 1 + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split `s` at top-level commas (outside quotes, at brace/bracket depth
/// 0), returning each part's `(offset within s, part)`.
fn split_flow_items(s: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (i, ch) in s.char_indices() {
        match quote {
            Some(q) if ch == q => quote = None,
            None if ch == '"' || ch == '\'' => quote = Some(ch),
            None if ch == '{' || ch == '[' => depth += 1,
            None if ch == '}' || ch == ']' => depth = depth.saturating_sub(1),
            None if ch == ',' && depth == 0 => {
                out.push((start, &s[start..i]));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push((start, &s[start..]));
    out
}

/// When `content` is a `key: value` pair — a plain-identifier key (or a
/// single layer of quotes), a colon, then end-of-line or whitespace —
/// return the key and, if the value is a scalar token, its `(start, end,
/// value)` with byte offsets relative to `base` (span includes quotes,
/// value does not; value shape per [`scalar_token`]).
#[allow(clippy::type_complexity)] // (key, Option<(start, end, value)>) pair
fn key_value(content: &str, base: usize) -> Option<(&str, Option<(usize, usize, String)>)> {
    let pad = content.len() - content.trim_start().len();
    let s = &content[pad..];
    let colon = s.find(':')?;
    let key_raw = &s[..colon];
    let key = key_raw
        .strip_prefix('"')
        .and_then(|k| k.strip_suffix('"'))
        .or_else(|| {
            key_raw
                .strip_prefix('\'')
                .and_then(|k| k.strip_suffix('\''))
        })
        .unwrap_or(key_raw);
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return None;
    }
    let after = &s[colon + 1..];
    if !after.is_empty() && !after.starts_with(' ') && !after.starts_with('\t') {
        return None;
    }
    let vpad = after.len() - after.trim_start().len();
    let value = scalar_token(&after[vpad..], base + pad + colon + 1 + vpad);
    Some((key, value))
}

/// Collect `project`/`ref` scalar pairs from a flow mapping's inside (pairs
/// = top-level comma pieces, one quote layer stripped) and push a flow
/// `IncludeRef` when both are present. A nested `{`/`[` or a stray closer
/// anywhere in the inside means no target.
fn push_include_flow_pairs(
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    inside: &str,
    inside_base: usize,
) {
    if inside.contains(['{', '[', ']', '}']) {
        return; // nested structure: no target
    }
    let mut project: Option<String> = None;
    let mut ref_tok: Option<(usize, usize, String)> = None;
    for (poff, pair) in split_flow_items(inside) {
        let (key, tok) = match key_value(pair, inside_base + poff) {
            Some(kv) => kv,
            None => continue,
        };
        match key {
            "project" => project = project.or_else(|| tok.map(|(_, _, v)| v)),
            "ref" => ref_tok = ref_tok.or(tok),
            _ => {}
        }
    }
    if let (Some(project), Some((start, end, value))) = (project, ref_tok) {
        targets.push(PinTarget::IncludeRef {
            line: line_idx,
            start: line_off + start,
            end: line_off + end,
            value,
            project,
            flow: true,
        });
    }
}

/// Scan a flow mapping whose `{` sits at `brace` in `line` (M3 Task 2);
/// see [`push_include_flow_pairs`].
fn push_include_flow_mapping(
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    brace: usize,
) {
    let Some(close) = flow_close(line, brace) else {
        return;
    };
    push_include_flow_pairs(
        targets,
        line_idx,
        line_off,
        &line[brace + 1..close],
        brace + 1,
    );
}

/// Scan a flow sequence whose `[` sits at `bracket` in `line` (M3 Task 2):
/// each item is a top-level comma piece; a `{...}` item is a flow mapping
/// (scanned per [`push_include_flow_pairs`]) and scalar or other items are
/// ignored.
fn push_include_flow_sequence(
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    bracket: usize,
) {
    let Some(close) = flow_close(line, bracket) else {
        return;
    };
    let inside = &line[bracket + 1..close];
    for (off, part) in split_flow_items(inside) {
        let item = part.trim();
        let Some(inner) = item
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
        else {
            continue; // scalar / other item → no target
        };
        // `inner` starts one byte after `item`'s first byte, which sits at
        // `bracket+1+off` plus the part's leading pad within the line.
        let item_base = bracket + 1 + off + (part.len() - item.len());
        push_include_flow_pairs(targets, line_idx, line_off, inner, item_base + 1);
    }
}

/// Consume one inner line of an open include block (M3 Task 2). A dash
/// line starts a sequence item (single-line flow mapping, block mapping,
/// or scalar — the last never a target); a non-dash line is a mapping key
/// at the item's key indent (sequence mode) or the block's child indent
/// (single-mapping mode).
fn handle_include_inner(
    targets: &mut Vec<PinTarget>,
    line_idx: usize,
    line_off: usize,
    line: &str,
    indent: usize,
    block: &mut IncludeBlock,
) {
    block.child_indent.get_or_insert(indent);
    if let Some(rest) = dash_item_rest(line) {
        // A new sequence item starts; the previous item's collection is
        // discarded (it lacked both keys).
        block.seq = Some(true);
        block.pending = None;
        block.item_key_indent = None;
        let content = &line[rest..];
        if content.starts_with('{') {
            push_include_flow_mapping(targets, line_idx, line_off, line, rest);
        } else if let Some((key, tok)) = key_value(content, rest) {
            // `- key: val` opens a block-mapping item whose keys sit at
            // the item's content column.
            block.item_key_indent = Some(rest);
            record_include_key(targets, block, line_idx, line_off, key, tok, false);
        }
        // Scalar items (string/remote/template/local) are not targets.
        return;
    }
    match block.seq {
        Some(true) => match block.item_key_indent {
            Some(ki) if ki < indent => {
                // Nested under the item's key: no target contribution.
            }
            Some(ki) if ki == indent => {
                if let Some((key, tok)) = key_value(&line[indent..], indent) {
                    record_include_key(targets, block, line_idx, line_off, key, tok, false);
                }
            }
            _ => {
                // A dedent to/below the item's key indent ends the item.
                block.pending = None;
                block.item_key_indent = None;
            }
        },
        _ => {
            // Single-mapping form; the first inner line sets the mode.
            if block.seq.is_none() {
                block.seq = Some(false);
            }
            if Some(indent) == block.child_indent
                && let Some((key, tok)) = key_value(&line[indent..], indent)
            {
                record_include_key(targets, block, line_idx, line_off, key, tok, false);
            }
            // Deeper lines are nested under a mapping key: no contribution.
        }
    }
}

/// Record a `project`/`ref` scalar key into the pending include mapping and
/// push the target as soon as both values are present (any key order).
/// A key without a scalar value (empty value, nested block) does not
/// contribute.
fn record_include_key(
    targets: &mut Vec<PinTarget>,
    block: &mut IncludeBlock,
    line_idx: usize,
    line_off: usize,
    key: &str,
    tok: Option<(usize, usize, String)>,
    flow: bool,
) {
    let Some(tok) = tok else {
        return;
    };
    let pending = block.pending.get_or_insert(PendingInclude {
        project: None,
        ref_tok: None,
        flow,
    });
    match key {
        "project" => {
            if pending.project.is_none() {
                pending.project = Some(tok.2);
            }
        }
        "ref" => {
            if pending.ref_tok.is_none() {
                pending.ref_tok = Some((line_idx, line_off + tok.0, line_off + tok.1, tok.2));
            }
        }
        _ => return,
    }
    if pending.project.is_some() && pending.ref_tok.is_some() {
        let project = pending.project.clone().expect("checked above");
        let (line, start, end, value) = pending.ref_tok.clone().expect("checked above");
        let flow = pending.flow;
        block.pending = None; // complete: fresh collection for the next item
        targets.push(PinTarget::IncludeRef {
            line,
            start,
            end,
            value,
            project,
            flow,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = include_str!("../tests/fixtures/ci-simple.yml");
    const RICH: &str = include_str!("../tests/fixtures/ci-rich.yml");

    const INCLUDE_FIXTURE: &str = include_str!("../tests/fixtures/ci-include.yml");

    #[test]
    fn image_targets_survive_enum_migration() {
        // M1/M2 image expectations, unchanged: ci-simple.yml …
        let t = discover(SIMPLE);
        let got: Vec<(usize, &str)> = t.iter().map(|x| (x.line(), x.value())).collect();
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
            let token = &SIMPLE[x.start()..x.end()];
            let unquoted = token.trim_matches(|c| c == '"' || c == '\'');
            assert_eq!(unquoted, x.value());
        }
        // … and ci-rich.yml.
        let t = discover(RICH);
        let got: Vec<(usize, &str)> = t
            .iter()
            .filter_map(|x| match x {
                PinTarget::Image { .. } => Some((x.line(), x.value())),
                // M3 include targets in this fixture are asserted separately.
                _ => None,
            })
            .collect();
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
    fn templated_value_is_still_a_target() {
        // Shell/YAML interpolation is a syntactic image candidate; the parse
        // rejection happens at resolve time (reported as `skipped
        // (unparseable reference)`), not here — discover must not silently
        // drop lines the user should see in the report.
        let text = "job:\n  image: ${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}\n";
        let t = discover(text);
        assert_eq!(t.len(), 1, "templated value must stay a target");
        assert_eq!(t[0].value(), "${APP_IMAGE}:${CI_COMMIT_SHORT_SHA}");
    }

    // ── M3 Task 2: classic include: discovery ───────────────────────

    #[test]
    fn include_flow_single_mapping() {
        // { project: x, ref: y } → target, flow=true, span on the ref value
        let text = "include: { project: gitlab-org/example, ref: main }  # flow single mapping\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "main");
                assert_eq!(project, "gitlab-org/example");
                assert_eq!(&text[*start..*end], "main");
                assert_eq!(*line, 0);
                assert!(*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_flow_sequence_mixed() {
        // [ {p+ref}, "str", {p only} ] → exactly 1 target (the flow mapping item)
        let text = "include: [ { project: g/p, ref: main }, \"local.yml\", { project: g/q } ]\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                value,
                project,
                flow,
                ..
            } => {
                assert_eq!(value, "main");
                assert_eq!(project, "g/p");
                assert!(*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_block_sequence_item() {
        // - project: x / ref: y (+file:) → target on the ref value, flow=false
        let text =
            "include:\n  - project: gitlab-org/example2\n    ref: v1.2.3\n    file: /ci/test.yml\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "v1.2.3");
                assert_eq!(project, "gitlab-org/example2");
                assert_eq!(&text[*start..*end], "v1.2.3");
                assert_eq!(*line, 2);
                assert!(!*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_block_single_mapping() {
        // ref before project → still a target; span on the ref line
        let text = "include:\n  ref: develop\n  project: gitlab-org/single\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "develop");
                assert_eq!(project, "gitlab-org/single");
                assert_eq!(&text[*start..*end], "develop");
                assert_eq!(*line, 1);
                assert!(!*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_string_and_remote_not_targets() {
        // "x.yml", remote:, template:, local: items → no IncludeRef at all
        let text = "include:\n  - \"x.yml\"\n  - remote: https://example.com/ci.yml\n  - template: something.yml\n  - local: /x.yml\n";
        assert!(discover(text).is_empty());
    }

    #[test]
    fn include_indented_not_a_target() {
        // Job-internal includes (indented) are out of scope: indent-0 only
        let text = "job:\n  include: { project: gitlab-org/indented, ref: main }\n";
        assert!(discover(text).is_empty());
    }

    #[test]
    fn include_templated_ref_still_a_target() {
        // ref: ${CI_REF} → still a target (syntactic discovery, no expansion)
        let text = "include:\n  project: g/p\n  ref: ${CI_REF}\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef { value, project, .. } => {
                assert_eq!(value, "${CI_REF}");
                assert_eq!(project, "g/p");
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn include_quoted_values() {
        // Quoted project/ref → target; span includes the quotes, value does not
        let text = "include:\n  project: \"g/p\"\n  ref: 'main'\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::IncludeRef {
                line,
                start,
                end,
                value,
                project,
                flow,
            } => {
                assert_eq!(value, "main");
                assert_eq!(project, "g/p");
                assert_eq!(&text[*start..*end], "'main'");
                assert_eq!(*line, 2);
                assert!(!*flow);
            }
            other => panic!("expected IncludeRef, got {other:?}"),
        }
    }

    #[test]
    fn combined_targets_sorted_by_line_then_start() {
        // image + component + include in one text: one global (line, start)
        // sort across all three kinds
        let text = "image: alpine:3.20\ncomponent: gitlab.com/g/p/name@1.0.0\ninclude: { project: g/p, ref: main }\ninclude:\n  project: g/p2\n  ref: v9\n";
        let t = discover(text);
        let got: Vec<(usize, usize, &str)> = t
            .iter()
            .map(|x| match x {
                PinTarget::Image { .. } => (x.line(), x.start(), "image"),
                PinTarget::Component { .. } => (x.line(), x.start(), "component"),
                PinTarget::IncludeRef { .. } => (x.line(), x.start(), "include"),
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, 7, "image"),
                (1, 30, "component"), // line off 19 + 11
                (2, 86, "include"),   // line off 56 + 30
                (5, 125, "include"),  // line off 118 + 7
            ]
        );
        assert_eq!(&text[t[2].start()..t[2].end()], "main");
        assert_eq!(&text[t[3].start()..t[3].end()], "v9");
    }

    #[test]
    fn include_fixture_all_forms() {
        // The fixture exercises every documented form: exactly the three
        // annotated targets, nothing else (indented include excluded).
        let t = discover(INCLUDE_FIXTURE);
        assert_eq!(t.len(), 3);
        let got: Vec<(usize, &str, &str, bool)> = t
            .iter()
            .map(|x| match x {
                PinTarget::IncludeRef { project, flow, .. } => {
                    (x.line(), x.value(), project.as_str(), *flow)
                }
                other => panic!("unexpected target: {other:?}"),
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "main", "gitlab-org/example", true),
                (3, "v1.2.3", "gitlab-org/example2", false),
                (10, "develop", "gitlab-org/single", false),
            ]
        );
    }

    #[test]
    fn rich_include_block_is_include_target() {
        // M3: ci-rich.yml's include block (project + ref) is now a target
        let t = discover(RICH);
        let got: Vec<(usize, &str, &str)> = t
            .iter()
            .filter_map(|x| match x {
                PinTarget::IncludeRef { project, .. } => {
                    Some((x.line(), x.value(), project.as_str()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(got, vec![(16, "v1.0.0", "group/proj")]);
    }

    #[test]
    fn script_items_are_not_targets() {
        let t = discover(RICH);
        assert!(t.iter().all(|x| x.value() != "echo hi"));
    }

    #[test]
    fn trailing_comment_preserved_in_spans() {
        let t = discover(RICH);
        let first = &t[0];
        assert_eq!(&RICH[first.start()..first.end()], "alpine:3.20");
    }

    #[test]
    fn component_top_level_target() {
        let text = "component: gitlab.com/g/sub/p/name@1.0.0\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::Component {
                value,
                parts,
                start,
                end,
                line,
            } => {
                assert_eq!(value, "gitlab.com/g/sub/p/name@1.0.0");
                let p = parts.as_ref().expect("parts");
                assert_eq!(
                    (
                        p.host.as_str(),
                        p.project.as_str(),
                        p.name.as_str(),
                        p.version.as_str()
                    ),
                    ("gitlab.com", "g/sub/p", "name", "1.0.0")
                );
                assert_eq!(&text[*start..*end], "gitlab.com/g/sub/p/name@1.0.0");
                assert_eq!(*line, 0);
            }
            other => panic!("expected Component, got {other:?}"),
        }
    }

    #[test]
    fn component_indented_is_not_a_target() {
        let text = "job:\n  component: gitlab.com/g/p/name@1.0.0\n";
        assert!(discover(text).is_empty());
    }

    #[test]
    fn component_unparseable_kept_with_none_parts() {
        for v in [
            "gitlab.com/name@1.0.0", // no project segment
            "gitlab.com/g/p/name",   // no '@'
            "${VAR}@1.0.0",          // templated host
            "gitlab.com/g/p/name@",  // empty version
        ] {
            let text = format!("component: {v}\n");
            let t = discover(&text);
            assert_eq!(t.len(), 1, "value {v} must stay a target");
            match &t[0] {
                PinTarget::Component { parts, value, .. } => {
                    assert!(parts.is_none(), "value {v} must have no parts");
                    assert_eq!(value, v);
                }
                other => panic!("expected Component, got {other:?}"),
            }
        }
    }

    #[test]
    fn component_quoted_and_comment() {
        let text = "component: \"gitlab.com/g/p/name@2.0\"  # from docs\n";
        let t = discover(text);
        assert_eq!(t.len(), 1);
        match &t[0] {
            PinTarget::Component {
                value, start, end, ..
            } => {
                assert_eq!(value, "gitlab.com/g/p/name@2.0");
                assert_eq!(&text[*start..*end], "\"gitlab.com/g/p/name@2.0\"");
            }
            other => panic!("expected Component, got {other:?}"),
        }
    }

    #[test]
    #[allow(clippy::type_complexity)] // (input, Option<(host, project, name, version)>) table
    fn parse_component_ref_table() {
        let cases: [(&str, Option<(&str, &str, &str, &str)>); 6] = [
            (
                "gitlab.com/g/p/name@1.0.0",
                Some(("gitlab.com", "g/p", "name", "1.0.0")),
            ),
            (
                "gitlab.com/a/b/c/d/name@v9",
                Some(("gitlab.com", "a/b/c/d", "name", "v9")),
            ),
            ("gitlab.com/name@1.0.0", None), // no project segment
            ("gitlab.com/g/p/name", None),   // no '@'
            ("gitlab.com/g/p/name@", None),  // empty version
            ("$CI_SERVER_FQDN/g/p/name@1.0.0", None), // templated host
        ];
        for (input, expected) in cases {
            let parsed = parse_component_ref(input);
            let got = parsed.as_ref().map(|p| {
                (
                    p.host.as_str(),
                    p.project.as_str(),
                    p.name.as_str(),
                    p.version.as_str(),
                )
            });
            assert_eq!(got, expected, "parse_component_ref({input})");
        }
    }
}
