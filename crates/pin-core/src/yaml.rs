//! Span-preserving YAML scan (spec §6.9; feature `yaml`).
//!
//! [`scan`] walks the events of `saphyr-parser` and returns every value node
//! with its path from the document root and its BYTE range in the original
//! text. The parser never writes: consumers match paths and edit by
//! splicing bytes (`edit`), so everything outside a reference stays as
//! written.
//!
//! Offsets: the parser counts CHARACTERS (a BOM is char 0), so one
//! char→byte table per file converts them; `Marker::col()` is never used.
//! A file that does not parse is a [`ScanError`] (sticky upstream: no
//! partial result, no fallback — D32).

use std::borrow::Cow;
use std::ops::Range;

use saphyr_parser::{Event, Parser, ScalarStyle, Span, Tag};

use crate::plugin::ScanError;
use crate::text::LineIndex;

/// One step of a path from the document root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    /// A mapping key (scalar keys only).
    Key(String),
    /// A sequence index.
    Index(usize),
    /// A non-scalar or alias key; its subtree is not scanned.
    Complex,
}

/// How a scalar is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Plain,
    Single,
    Double,
    /// `|` or `>`.
    Block,
}

/// A scalar value node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalarRef {
    /// Value path from the document root; keys are not nodes.
    pub path: Vec<PathSeg>,
    /// 0-based document index.
    pub doc: usize,
    /// 0-based line of `span.start`.
    pub line: usize,
    /// BYTE range of the token including quotes.
    ///
    /// An implicit null (`key:` or flow `{k}` without a value) is an empty
    /// `Plain` scalar with `value == ""` and an EMPTY span `s..s` (a plain
    /// token is never empty, and the parser's span for it is the NEXT
    /// token): block → at the key's `:` (key line); flow → start of the
    /// next token (`}` / `,`); with properties → start of the properties.
    pub span: Range<usize>,
    pub style: Style,
    /// The decoded value (from the masked text when masking).
    pub value: String,
    /// The anchor id when this scalar defines one.
    pub anchor: Option<usize>,
    /// Id of the enclosing collection (ids from 1, in event order); 0 when
    /// the document is this scalar. Groups mapping siblings.
    pub parent: usize,
    /// The enclosing collection is a flow collection (or nested in one).
    pub in_flow: bool,
    /// A non-empty span overlaps a masked `{{…}}`, or (plain) touches one
    /// across blanks: the value is a fragment of a template, never a
    /// complete reference.
    pub templated: bool,
}

/// A value node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Scalar(ScalarRef),
    /// `*name`; never a target.
    Alias {
        path: Vec<PathSeg>,
        id: usize,
        span: Range<usize>,
        parent: usize,
    },
    /// A node with a non-core tag (`!reference`); its subtree is not
    /// emitted.
    Tagged {
        path: Vec<PathSeg>,
        tag: String,
        parent: usize,
    },
}

/// Options of [`scan`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanOpts {
    /// Blank every `{{…}}` before parsing (Go-templated files).
    pub mask_templates: bool,
}

/// A collection being walked.
enum Frame {
    Map {
        id: usize,
        flow: bool,
        key: Option<PathSeg>,
        want_key: bool,
    },
    Seq {
        id: usize,
        flow: bool,
        idx: usize,
    },
}

impl Frame {
    fn id(&self) -> usize {
        match self {
            Frame::Map { id, .. } | Frame::Seq { id, .. } => *id,
        }
    }

    fn flow(&self) -> bool {
        match self {
            Frame::Map { flow, .. } | Frame::Seq { flow, .. } => *flow,
        }
    }
}

fn is_blank(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Skip spaces, tabs, line breaks and `#` comments from `s`, never past `e`.
fn skip_blanks(src: &[u8], mut s: usize, e: usize) -> usize {
    while s < e {
        match src.get(s) {
            Some(b' ' | b'\t' | b'\r' | b'\n') => s += 1,
            Some(b'#') => {
                while s < e && src.get(s) != Some(&b'\n') {
                    s += 1;
                }
            }
            _ => break,
        }
    }
    s
}

/// Skip node properties (`&anchor`, `!tag`) and what follows them (blanks,
/// breaks, comments) from `s`, never past `e` (the parser's span starts at
/// the properties).
fn skip_props(src: &[u8], mut s: usize, e: usize) -> usize {
    while s < e && matches!(src.get(s), Some(b'&' | b'!')) {
        while s < e && src.get(s).is_some_and(|b| !is_blank(*b)) {
            s += 1;
        }
        s = skip_blanks(src, s, e);
    }
    s
}

/// The exact token bytes of a scalar whose parser span is `s..e`.
///
/// The parser's span starts at the properties, and a quoted scalar's runs
/// past the closing quote over blanks and a comment, so both are re-derived
/// from the text. `has_props`: the event carries an anchor or a tag; only
/// then is anything skipped (block content may itself start with `!`/`&`).
/// Every scan stops at `e`; `None` = no such token (internal error).
fn token_span(
    src: &[u8],
    s: usize,
    e: usize,
    style: Style,
    has_props: bool,
) -> Option<Range<usize>> {
    let mut s = s;
    if has_props {
        s = skip_props(src, s, e);
    }
    match style {
        Style::Block => {
            if has_props {
                // header line (indicator, chomping, comment, break), then
                // the indentation and blank lines before the content
                while s < e && src.get(s) != Some(&b'\n') {
                    s += 1;
                }
                s = (s + 1).min(e);
                while s < e && src.get(s).is_some_and(|b| is_blank(*b)) {
                    s += 1;
                }
            }
            let content = src.get(s..e)?;
            let len = content
                .iter()
                .rposition(|b| !is_blank(*b))
                .map_or(0, |i| i + 1);
            Some(s..s + len)
        }
        Style::Single => {
            let mut i = s + 1;
            while i < e {
                if src.get(i) == Some(&b'\'') {
                    if src.get(i + 1) == Some(&b'\'') {
                        i += 2;
                        continue;
                    }
                    return Some(s..i + 1);
                }
                i += 1;
            }
            None
        }
        Style::Double => {
            let mut i = s + 1;
            while i < e {
                match src.get(i) {
                    Some(b'\\') => i += 2,
                    Some(b'"') => return Some(s..i + 1),
                    _ => i += 1,
                }
            }
            None
        }
        Style::Plain => Some(s..e),
    }
}

/// The tag as written when it is not a YAML core-schema tag (`!reference`,
/// `!custom`); core tags (`!!str`, `!!seq`) are transparent.
fn opaque_tag(tag: &Option<Cow<'_, Tag>>) -> Option<String> {
    tag.as_ref()
        .filter(|t| !t.is_yaml_core_schema())
        .map(|t| t.to_string())
}

/// Blank every `{{…}}` (non-greedy, may span lines; an unclosed `{{` is
/// left alone) so Go-templated files parse: byte offsets and line numbers
/// stay exact because every byte becomes a space except `\n` and `\r`. Also
/// returns the masked ranges.
fn mask_templates(text: &str) -> (String, Vec<Range<usize>>) {
    let mut ranges = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find("{{").map(|i| from + i) {
        let Some(close) = text[open + 2..].find("}}").map(|i| open + 2 + i) else {
            break;
        };
        ranges.push(open..close + 2);
        from = close + 2;
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for r in &ranges {
        out.push_str(&text[last..r.start]);
        for c in text[r.clone()].chars() {
            if matches!(c, '\n' | '\r') {
                out.push(c);
            } else {
                out.extend(std::iter::repeat_n(' ', c.len_utf8()));
            }
        }
        last = r.end;
    }
    out.push_str(&text[last..]);
    (out, ranges)
}

struct Walker<'a> {
    /// The original text: its char boundaries bound every span.
    text: &'a str,
    /// The text the parser saw (same bytes unless masked).
    src: &'a [u8],
    /// Parser char index → byte offset in `src`.
    c2b: &'a [usize],
    lines: &'a LineIndex,
    /// Masked `{{…}}` byte ranges (empty unless masking).
    mask: &'a [Range<usize>],
    stack: Vec<Frame>,
    next_id: usize,
    docs_seen: usize,
    /// Inside a subtree that emits nothing: `(open collections, the subtree
    /// was a mapping key)`.
    skip: Option<(usize, bool)>,
    nodes: Vec<Node>,
}

impl Walker<'_> {
    /// The byte range of a parser span, or an internal error.
    fn bytes(&self, span: &Span) -> Result<(usize, usize), ScanError> {
        let s = self.c2b.get(span.start.index()).copied();
        let e = self.c2b.get(span.end.index()).copied();
        match (s, e) {
            (Some(s), Some(e))
                if s <= e && self.text.is_char_boundary(s) && self.text.is_char_boundary(e) =>
            {
                Ok((s, e))
            }
            _ => Err(ScanError {
                line: Some(span.start.line()),
                msg: format!("internal: bad span at byte {}", span.start.index()),
            }),
        }
    }

    /// A non-empty token overlaps a masked `{{…}}`. A plain scalar counts
    /// the blanks around it first: the masked bytes are blanks, so the
    /// parser ends the token where the template begins (`x-{{ .v }}`) and
    /// starts it where the template ends (`{{ .r }}/x`), but Go renders one
    /// scalar. Blanks stay on the line; a flow separator ends the scalar.
    fn templated(&self, range: &Range<usize>, style: Style) -> bool {
        if range.is_empty() || self.mask.is_empty() {
            return false;
        }
        let blank = |i: usize| matches!(self.src.get(i), Some(b' ' | b'\t'));
        let (mut start, mut end) = (range.start, range.end);
        if style == Style::Plain {
            while start > 0 && blank(start - 1) {
                start -= 1;
            }
            while blank(end) {
                end += 1;
            }
        }
        self.mask.iter().any(|m| m.start < end && start < m.end)
    }

    fn at_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Map { want_key: true, .. }))
    }

    /// A value node is complete: the parent expects its next key / item.
    fn advance(&mut self) {
        match self.stack.last_mut() {
            Some(Frame::Map { want_key, .. }) => *want_key = true,
            Some(Frame::Seq { idx, .. }) => *idx += 1,
            None => {}
        }
    }

    fn path(&self) -> Vec<PathSeg> {
        self.stack
            .iter()
            .map(|f| match f {
                Frame::Map { key, .. } => key.clone().unwrap_or(PathSeg::Complex),
                Frame::Seq { idx, .. } => PathSeg::Index(*idx),
            })
            .collect()
    }

    fn parent(&self) -> usize {
        self.stack.last().map_or(0, Frame::id)
    }

    fn in_flow(&self) -> bool {
        self.stack.last().is_some_and(Frame::flow)
    }

    /// A non-scalar or alias key: the next value belongs to a key no
    /// consumer matches.
    fn complex_key(&mut self) {
        if let Some(Frame::Map { key, want_key, .. }) = self.stack.last_mut() {
            *key = Some(PathSeg::Complex);
            *want_key = false;
        }
    }

    fn collection(
        &mut self,
        is_map: bool,
        anchor: usize,
        tag: &Option<Cow<'_, Tag>>,
        span: &Span,
    ) -> Result<(), ScanError> {
        if self.at_key() {
            self.complex_key();
            self.skip = Some((1, true));
            return Ok(());
        }
        if let Some(tag) = opaque_tag(tag) {
            self.nodes.push(Node::Tagged {
                path: self.path(),
                tag,
                parent: self.parent(),
            });
            self.skip = Some((1, false));
            return Ok(());
        }
        let (mut s, e) = self.bytes(span)?;
        if anchor > 0 || tag.is_some() {
            s = skip_props(self.src, s, e);
        }
        let flow = self.in_flow() || (s < e && matches!(self.src.get(s), Some(b'{' | b'[')));
        let id = self.next_id;
        self.next_id += 1;
        self.stack.push(if is_map {
            Frame::Map {
                id,
                flow,
                key: None,
                want_key: true,
            }
        } else {
            Frame::Seq { id, flow, idx: 0 }
        });
        Ok(())
    }

    fn scalar(
        &mut self,
        value: Cow<'_, str>,
        style: ScalarStyle,
        anchor: usize,
        tag: &Option<Cow<'_, Tag>>,
        span: &Span,
    ) -> Result<(), ScanError> {
        if self.at_key() {
            if let Some(Frame::Map { key, want_key, .. }) = self.stack.last_mut() {
                *key = Some(PathSeg::Key(value.into_owned()));
                *want_key = false;
            }
            return Ok(());
        }
        if let Some(tag) = opaque_tag(tag) {
            self.nodes.push(Node::Tagged {
                path: self.path(),
                tag,
                parent: self.parent(),
            });
            self.advance();
            return Ok(());
        }
        let (s, e) = self.bytes(span)?;
        let style = match style {
            ScalarStyle::Plain => Style::Plain,
            ScalarStyle::SingleQuoted => Style::Single,
            ScalarStyle::DoubleQuoted => Style::Double,
            ScalarStyle::Literal | ScalarStyle::Folded => Style::Block,
        };
        let range = if style == Style::Plain && value.is_empty() {
            s..s
        } else {
            let has_props = anchor > 0 || tag.is_some();
            token_span(self.src, s, e, style, has_props)
                .filter(|r| {
                    self.text.is_char_boundary(r.start) && self.text.is_char_boundary(r.end)
                })
                .ok_or_else(|| ScanError {
                    line: Some(span.start.line()),
                    msg: format!("internal: bad span at byte {}", span.start.index()),
                })?
        };
        let templated = self.templated(&range, style);
        self.nodes.push(Node::Scalar(ScalarRef {
            path: self.path(),
            doc: self.docs_seen.saturating_sub(1),
            line: self.lines.line_of(range.start),
            span: range,
            style,
            value: value.into_owned(),
            anchor: (anchor > 0).then_some(anchor),
            parent: self.parent(),
            in_flow: self.in_flow(),
            templated,
        }));
        self.advance();
        Ok(())
    }

    fn event(&mut self, ev: Event<'_>, span: &Span) -> Result<(), ScanError> {
        if let Some((depth, was_key)) = self.skip {
            match ev {
                Event::MappingStart(..) | Event::SequenceStart(..) => {
                    self.skip = Some((depth + 1, was_key));
                }
                Event::MappingEnd | Event::SequenceEnd if depth > 1 => {
                    self.skip = Some((depth - 1, was_key));
                }
                Event::MappingEnd | Event::SequenceEnd => {
                    self.skip = None;
                    if !was_key {
                        self.advance();
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        match ev {
            Event::DocumentStart(_) => {
                self.docs_seen += 1;
                self.stack.clear();
            }
            Event::Scalar(value, style, anchor, tag) => {
                self.scalar(value, style, anchor, &tag, span)?;
            }
            Event::Alias(id) => {
                if self.at_key() {
                    self.complex_key();
                } else {
                    let (s, e) = self.bytes(span)?;
                    self.nodes.push(Node::Alias {
                        path: self.path(),
                        id,
                        span: s..e,
                        parent: self.parent(),
                    });
                    self.advance();
                }
            }
            Event::MappingStart(anchor, tag) => self.collection(true, anchor, &tag, span)?,
            Event::SequenceStart(anchor, tag) => self.collection(false, anchor, &tag, span)?,
            Event::MappingEnd | Event::SequenceEnd => {
                self.stack.pop();
                self.advance();
            }
            _ => {}
        }
        Ok(())
    }
}

/// Whether the byte `at` lies strictly inside the token of a scalar or alias
/// in `nodes`: a line break there is part of a multi-line scalar (or of a
/// quoted one's escape), so nothing — a comment — may be inserted at it
/// (review M2). Keys and the subtrees of [`Node::Tagged`] are not nodes
/// and not covered.
pub fn inside_token(nodes: &[Node], at: usize) -> bool {
    nodes.iter().any(|n| match n {
        Node::Scalar(ScalarRef { span, .. }) | Node::Alias { span, .. } => {
            span.start < at && at < span.end
        }
        Node::Tagged { .. } => false,
    })
}

/// The parser's message for a tab after `:` (and no space): YAML allows a
/// tab as separation there, saphyr does not (review H1).
const TAB_AFTER_COLON: &str = "':' must be followed by a valid YAML whitespace";

/// A failed parse: the error, and — when it is [`TAB_AFTER_COLON`] — the
/// parser's char index of the error (just after the tab run).
type Failure = (ScanError, Option<usize>);

/// One pass over the events of `src` (the text the parser sees; same bytes
/// and chars as `text` except for masking and blanked tabs).
fn walk(
    text: &str,
    src: &str,
    c2b: &[usize],
    lines: &LineIndex,
    mask: &[Range<usize>],
) -> Result<Vec<Node>, Failure> {
    let mut walker = Walker {
        text,
        src: src.as_bytes(),
        c2b,
        lines,
        mask,
        stack: Vec::new(),
        next_id: 1,
        docs_seen: 0,
        skip: None,
        nodes: Vec::new(),
    };
    for item in Parser::new_from_str(src) {
        let (ev, span) = item.map_err(|e| {
            let marker = (e.info() == TAB_AFTER_COLON).then(|| e.marker().index());
            let err = ScanError {
                line: Some(e.marker().line()),
                msg: format!("invalid YAML: {}", e.info()),
            };
            (err, marker)
        })?;
        walker.event(ev, &span).map_err(|e| (e, None))?;
    }
    Ok(walker.nodes)
}

/// `src` with the tab run that ends at char `marker` blanked (tab → space:
/// same length, so every offset stays); `None` when no tab precedes it.
fn blank_tabs_before(src: &str, c2b: &[usize], marker: usize) -> Option<String> {
    let end = *c2b.get(marker)?;
    let bytes = src.as_bytes();
    let mut start = end;
    while start > 0 && bytes.get(start - 1) == Some(&b'\t') {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let mut out = String::with_capacity(src.len());
    out.push_str(&src[..start]);
    out.extend(std::iter::repeat_n(' ', end - start));
    out.push_str(&src[end..]);
    Some(out)
}

/// The first bare CR (a `\r` not followed by `\n`) as a [`ScanError`]: the
/// parser breaks lines at a lone `\r`, the line model does not (review L1).
fn bare_cr(text: &str) -> Option<ScanError> {
    let b = text.as_bytes();
    let at = (0..b.len()).find(|&i| b[i] == b'\r' && b.get(i + 1) != Some(&b'\n'))?;
    Some(ScanError {
        line: Some(LineIndex::new(text).line_of(at) + 1),
        msg: "bare CR line endings are not supported".into(),
    })
}

/// Every value node of `text` in event order.
///
/// `Err` on the first parser error (sticky upstream): no partial result.
/// The message is `invalid YAML: {parser info}`, `line` the 1-based line of
/// the error. A bare CR (`\r` not followed by `\n`) is an error too: lines
/// end at `\n` or `\r\n`.
///
/// With `opts.mask_templates` the parser sees the text with every `{{…}}`
/// blanked (see `mask_templates`): `value`s come from the masked text,
/// `span`s are byte ranges of the original, and a scalar whose non-empty
/// span overlaps a masked range is `templated` (a plain scalar also when
/// only blanks separate it from the range: `x-{{ .v }}`, `{{ .r }}/x`).
///
/// A tab after `key:` is separator whitespace in YAML but an error for the
/// parser: it is blanked (tab → space, offsets unchanged) in the scan copy
/// and the parse retried, once per flagged tab at most and within
/// [`RETRY_BUDGET`]. The spans index the original text; block indentation
/// tabs stay errors.
pub fn scan(text: &str, opts: ScanOpts) -> Result<Vec<Node>, ScanError> {
    scan_within(text, opts, RETRY_BUDGET)
}

/// Bytes of text the tab retries may re-parse in all (each retry parses
/// the file again, up to the next flagged tab): bounds a file full of tab
/// separators to seconds. Beyond it the parser's error is returned.
const RETRY_BUDGET: usize = 64 << 20;

fn scan_within(text: &str, opts: ScanOpts, mut budget: usize) -> Result<Vec<Node>, ScanError> {
    if let Some(err) = bare_cr(text) {
        return Err(err);
    }
    let (mut src, mask) = if opts.mask_templates {
        mask_templates(text)
    } else {
        (text.to_string(), Vec::new())
    };
    // the parser counts the chars of the text it sees; a blanked tab is
    // one char for one char, so the table serves every retry
    let mut c2b: Vec<usize> = src.char_indices().map(|(b, _)| b).collect();
    c2b.push(src.len());
    let lines = LineIndex::new(text);
    loop {
        match walk(text, &src, &c2b, &lines, &mask) {
            Ok(nodes) => return Ok(nodes),
            // every retry blanks at least one tab: the loop ends
            Err((err, Some(marker))) if budget >= src.len() => {
                let Some(next) = blank_tabs_before(&src, &c2b, marker) else {
                    return Err(err);
                };
                budget -= src.len();
                src = next;
            }
            Err((err, _)) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    /// `job.services[1].name`, `[0]`, `ä[1]`.
    fn path_str(p: &[PathSeg]) -> String {
        let mut out = String::new();
        for seg in p {
            match seg {
                PathSeg::Key(k) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(k);
                }
                PathSeg::Index(i) => write!(out, "[{i}]").unwrap(),
                PathSeg::Complex => out.push('?'),
            }
        }
        out
    }

    type Row = (String, String, String, usize);

    fn row(path: &str, value: &str, token: &str, line: usize) -> Row {
        (path.into(), value.into(), token.into(), line)
    }

    /// `(path, value, text[span], line)` of every scalar, in event order.
    fn scalars(t: &str) -> Vec<Row> {
        scan(t, ScanOpts::default())
            .unwrap()
            .into_iter()
            .filter_map(|n| match n {
                Node::Scalar(s) => Some((
                    path_str(&s.path),
                    s.value.clone(),
                    t[s.span.clone()].to_string(),
                    s.line,
                )),
                _ => None,
            })
            .collect()
    }

    fn scalar_at(t: &str, path: &str) -> ScalarRef {
        scan(t, ScanOpts::default())
            .unwrap()
            .into_iter()
            .find_map(|n| match n {
                Node::Scalar(s) if path_str(&s.path) == path => Some(s),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no scalar at {path}"))
    }

    #[test]
    fn scan_paths_values_and_plain_spans() {
        let t = "image: alpine:3.20  # c\njob:\n  services:\n    - redis:7\n    - name: node:20\n";
        assert_eq!(
            scalars(t),
            [
                row("image", "alpine:3.20", "alpine:3.20", 0),
                row("job.services[0]", "redis:7", "redis:7", 3),
                row("job.services[1].name", "node:20", "node:20", 4),
            ]
        );
    }

    #[test]
    fn scan_documents_reset_paths() {
        let t = "a: 1\n---\nb: 2\n...\n---\n- c\n";
        let got: Vec<_> = scan(t, ScanOpts::default())
            .unwrap()
            .into_iter()
            .map(|n| match n {
                Node::Scalar(s) => (s.doc, path_str(&s.path), s.value),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            got,
            [
                (0, "a".to_string(), "1".to_string()),
                (1, "b".to_string(), "2".to_string()),
                (2, "[0]".to_string(), "c".to_string()),
            ]
        );
        assert_eq!(scan("", ScanOpts::default()), Ok(vec![]));
        assert_eq!(scan("# only\n", ScanOpts::default()), Ok(vec![]));
    }

    #[test]
    fn scan_char_to_byte_non_ascii_and_bom() {
        let t = "\u{FEFF}k: x\nä: [y, 🚀z]\nimg: alpine:3.20\n";
        let got = scalars(t);
        let tokens: Vec<&str> = got.iter().map(|r| r.2.as_str()).collect();
        assert_eq!(tokens, ["x", "y", "🚀z", "alpine:3.20"]);
        assert_eq!(got[1].0, "ä[0]");
        assert_eq!(got[2].0, "ä[1]");
        let s = scalar_at(t, "img");
        let at = t.find("alpine:3.20").unwrap();
        assert_eq!(s.span, at..at + 11);
    }

    #[test]
    fn scan_crlf_spans_exclude_cr() {
        let t = "a: x\r\nb:\r\n  - y\r\n";
        assert_eq!(
            scalars(t),
            [row("a", "x", "x", 0), row("b[0]", "y", "y", 2)]
        );
    }

    #[test]
    fn scan_parent_ids_and_flow_flag() {
        let t = "include: {project: a, ref: b}\ninclude:\n  - project: c\n    ref: d\n";
        let nodes: Vec<ScalarRef> = scan(t, ScanOpts::default())
            .unwrap()
            .into_iter()
            .filter_map(|n| match n {
                Node::Scalar(s) => Some(s),
                _ => None,
            })
            .collect();
        let by_value = |v: &str| nodes.iter().find(|s| s.value == v).unwrap();
        let (a, b, c, d) = (by_value("a"), by_value("b"), by_value("c"), by_value("d"));
        assert_eq!(
            a.path,
            [
                PathSeg::Key("include".into()),
                PathSeg::Key("project".into())
            ]
        );
        assert_eq!(a.parent, b.parent);
        assert_eq!(c.parent, d.parent);
        assert_ne!(a.parent, c.parent);
        assert_ne!(a.parent, 0);
        assert!(a.in_flow && b.in_flow);
        assert!(!c.in_flow && !d.in_flow);
    }

    #[test]
    fn scan_error_has_line_and_msg() {
        assert_eq!(
            scan("a: 1\nb: c: d\n", ScanOpts::default()),
            Err(ScanError {
                line: Some(2),
                msg: "invalid YAML: mapping values are not allowed in this context".into()
            })
        );
    }

    /// Review H1: an implicit null (`key:`, flow `{k}`) is an empty plain
    /// scalar with an EMPTY span; the parser's own span is the NEXT token.
    #[test]
    fn empty_plain_value_has_empty_span() {
        // block: at the key's `:`, on the key's line
        let s = scalar_at("image:\n  digest:\n", "image.digest");
        assert_eq!((s.value.as_str(), s.style), ("", Style::Plain));
        assert_eq!((s.span, s.line), (15..15, 1));

        // flow, last key: the parser span is the `}`
        let t = "include: {project: p, ref}\n";
        assert_eq!(scalar_at(t, "include.project").span, 19..20);
        let r = scalar_at(t, "include.ref");
        assert_eq!((r.value.as_str(), r.span), ("", 25..25));

        // flow, first key: the parser span is `, `
        let t = "include: {ref, project: p}\n";
        assert_eq!(scalar_at(t, "include.ref").span, 13..13);
        assert_eq!(scalar_at(t, "include.project").span, 24..25);

        // with properties: start of the properties
        let s = scalar_at("a: &x\n", "a");
        assert_eq!((s.value.as_str(), s.span), ("", 3..3));
        assert!(s.anchor.is_some());
    }

    #[test]
    fn quoted_spans_end_at_closing_quote() {
        let t = "a: \"x:1\" # c\nb: 'it''s:1'   # c\nc: [\"d:1\" , 'e:1' ]\nd: \"q\\\"q\"  \n";
        let got = scalars(t);
        let tokens: Vec<&str> = got.iter().map(|r| r.2.as_str()).collect();
        assert_eq!(
            tokens,
            ["\"x:1\"", "'it''s:1'", "\"d:1\"", "'e:1'", "\"q\\\"q\""]
        );
        let values: Vec<&str> = got.iter().map(|r| r.1.as_str()).collect();
        assert_eq!(values, ["x:1", "it's:1", "d:1", "e:1", "q\"q"]);
    }

    #[test]
    fn properties_are_skipped() {
        let t = "a: &x v1\nb: !!str v2\nc: &y !!str \"v3\"\nd: !!str\n  v4\n";
        let tokens: Vec<(String, usize)> = scalars(t).into_iter().map(|r| (r.2, r.3)).collect();
        assert_eq!(
            tokens,
            [
                ("v1".to_string(), 0),
                ("v2".to_string(), 1),
                ("\"v3\"".to_string(), 2),
                ("v4".to_string(), 4),
            ]
        );
        let anchors: Vec<bool> = ["a", "b", "c", "d"]
            .iter()
            .map(|p| scalar_at(t, p).anchor.is_some())
            .collect();
        assert_eq!(anchors, [true, false, true, false]);
    }

    #[test]
    fn aliases_are_alias_nodes() {
        let t = "a: &x v\nb: *x\nc:\n  <<: *x\n";
        let nodes = scan(t, ScanOpts::default()).unwrap();
        assert_eq!(nodes.len(), 3);
        assert!(matches!(&nodes[0], Node::Scalar(s) if path_str(&s.path) == "a"));
        let aliases: Vec<(String, &str, usize)> = nodes[1..]
            .iter()
            .map(|n| match n {
                Node::Alias {
                    path, span, parent, ..
                } => (path_str(path), &t[span.clone()], *parent),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            aliases,
            [("b".to_string(), "*x", 1), ("c.<<".to_string(), "*x", 2),]
        );
        // the alias refers to the anchor id of the definition
        let Node::Scalar(def) = &nodes[0] else {
            unreachable!()
        };
        assert!(matches!(&nodes[1], Node::Alias { id, .. } if Some(*id) == def.anchor));
    }

    #[test]
    fn local_tags_are_opaque_core_tags_transparent() {
        let t = "a: !reference [.b, c]\nd: !!seq [e]\nf: !custom g\n";
        let nodes = scan(t, ScanOpts::default()).unwrap();
        assert_eq!(nodes.len(), 3, "{nodes:?}");
        assert!(matches!(
            &nodes[0],
            Node::Tagged { path, tag, parent: 1 }
                if path_str(path) == "a" && tag == "!reference"
        ));
        assert!(matches!(
            &nodes[1],
            Node::Scalar(s) if path_str(&s.path) == "d[0]" && s.value == "e" && s.parent == 2
        ));
        assert!(matches!(
            &nodes[2],
            Node::Tagged { path, tag, parent: 1 }
                if path_str(path) == "f" && tag == "!custom"
        ));
    }

    /// Review H1: a tab after `key:` is separator whitespace (valid YAML,
    /// accepted by GitLab) that saphyr rejects. `scan` blanks the flagged
    /// tab in a scan copy and retries; spans index the ORIGINAL text and
    /// every other tab keeps its meaning.
    #[test]
    fn tab_after_colon_is_a_separator() {
        let t = "variables:\n  FOO:\tbar\nimage:\tnginx:1.25\njob:\n  image:\t\talpine:3.20\nf: {a:\tb, c: d}\nq: \"x:\ty\"\n";
        assert_eq!(
            scalars(t),
            [
                row("variables.FOO", "bar", "bar", 1),
                row("image", "nginx:1.25", "nginx:1.25", 2),
                row("job.image", "alpine:3.20", "alpine:3.20", 4),
                row("f.a", "b", "b", 5),
                row("f.c", "d", "d", 5),
                // a tab inside a scalar is not touched
                row("q", "x:\ty", "\"x:\ty\"", 6),
            ]
        );
        let s = scalar_at(t, "image");
        assert_eq!(s.span, t.find("nginx").unwrap()..t.find("\njob").unwrap());
    }

    /// Review H1: the retry is not a general tab forgiveness: block
    /// indentation and shapes that stay invalid with a space are still
    /// errors, and the message is the parser's.
    #[test]
    fn tab_indentation_and_invalid_shapes_stay_errors() {
        for t in [
            "job:\n\timage: x\n",
            "a:\n\t- b\n",
            "a: |\n\tx\n",
            "a:\t- b\n",
            "a:\tb\nc: d: e\n",
        ] {
            let e = scan(t, ScanOpts::default()).unwrap_err();
            assert!(e.msg.starts_with("invalid YAML: "), "{t:?}: {e:?}");
        }
        let e = scan("job:\n\timage: x\n", ScanOpts::default()).unwrap_err();
        assert_eq!(
            e,
            ScanError {
                line: Some(2),
                msg: "invalid YAML: tabs disallowed within this context (block indentation)".into()
            }
        );
        // the error of the retried text is reported at its own line
        let e = scan("a:\tb\nc: d: e\n", ScanOpts::default()).unwrap_err();
        assert_eq!(e.line, Some(2));
    }

    /// Review H1: one retry per flagged tab, within a budget of re-parsed
    /// bytes; also with masking; spans stay on the original text.
    #[test]
    fn many_tab_separators_and_masking() {
        let mut t = String::new();
        for i in 0..200 {
            writeln!(t, "k{i}:\tv{i}").unwrap();
        }
        let nodes = scan(&t, ScanOpts::default()).unwrap();
        assert_eq!(nodes.len(), 200);
        // out of budget: the parser's error, at the first tab not retried
        let small = "a:\tb\nc:\td\ne:\tf\n";
        let opts = ScanOpts::default();
        assert!(scan_within(small, opts, small.len() * 3).is_ok());
        let e = scan_within(small, opts, small.len() * 2).unwrap_err();
        assert_eq!(
            e,
            ScanError {
                line: Some(3),
                msg: "invalid YAML: ':' must be followed by a valid YAML whitespace".into()
            }
        );
        let masked = "a:\tx-{{ .y }}\nb:\t\"q\"\n";
        let nodes = scan(
            masked,
            ScanOpts {
                mask_templates: true,
            },
        )
        .unwrap();
        let [Node::Scalar(a), Node::Scalar(b)] = &nodes[..] else {
            panic!("{nodes:?}")
        };
        assert_eq!(&masked[a.span.clone()], "x-");
        assert_eq!(&masked[b.span.clone()], "\"q\"");
    }

    /// Review M2: a block scalar's span is its content (end trimmed), also
    /// with properties; the style is kept.
    #[test]
    fn block_scalars_keep_style_and_content_span() {
        let t = "a: |\n  x\nb: >-\n  y\nc: |\n  !x\nd: &k >-\n  z\ne: !!str |\n  w\n  w2\n";
        let got = scalars(t);
        assert_eq!(
            got,
            [
                row("a", "x\n", "x", 1),
                row("b", "y", "y", 3),
                row("c", "!x\n", "!x", 5),
                row("d", "z", "z", 7),
                row("e", "w\nw2\n", "w\n  w2", 9),
            ]
        );
        for p in ["a", "b", "c", "d", "e"] {
            assert_eq!(scalar_at(t, p).style, Style::Block, "{p}");
        }
        assert!(scalar_at(t, "d").anchor.is_some());
    }

    /// P-mask: same length, line breaks kept, bytes outside the ranges kept,
    /// every other byte inside a range blanked.
    fn check_mask(t: &str, masked: &str, ranges: &[Range<usize>]) {
        assert_eq!(masked.len(), t.len());
        for (i, (a, b)) in t.bytes().zip(masked.bytes()).enumerate() {
            let inside = ranges.iter().any(|r| r.contains(&i));
            let want = if a == b'\n' || a == b'\r' || !inside {
                a
            } else {
                b' '
            };
            assert_eq!(b, want, "byte {i} of {t:?}");
        }
    }

    #[test]
    fn mask_keeps_offsets_and_line_breaks() {
        let t = "a: {{ .x }}\n{{- if y\n }}\nb: \"r:{{ ü }}\"\nc: {{ open\n";
        let (masked, ranges) = mask_templates(t);
        let span = |needle: &str| {
            let at = t.find(needle).unwrap();
            at..at + needle.len()
        };
        assert_eq!(
            ranges,
            [span("{{ .x }}"), span("{{- if y\n }}"), span("{{ ü }}")]
        );
        check_mask(t, &masked, &ranges);
        assert!(masked.ends_with("c: {{ open\n"), "{masked:?}");
        assert_eq!(masked.lines().count(), t.lines().count());
    }

    #[test]
    fn masked_scalars_are_templated() {
        let t = "img: \"r:{{ .t }}\"\nplain: r:1\n{{- if .x }}\nother: v\n{{- end }}\n";
        let nodes = scan(
            t,
            ScanOpts {
                mask_templates: true,
            },
        )
        .unwrap();
        let find = |p: &str| {
            nodes
                .iter()
                .find_map(|n| match n {
                    Node::Scalar(s) if path_str(&s.path) == p => Some(s),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no scalar at {p}"))
        };
        let img = find("img");
        assert!(img.templated);
        assert_eq!(&t[img.span.clone()], "\"r:{{ .t }}\"");
        assert!(!find("plain").templated);
        assert!(!find("other").templated);
        assert_eq!(find("other").line, 3);
    }

    /// Review L1: the parser breaks lines at a lone `\r`, the line model
    /// (`LineIndex`, comment placement, multi-line checks) does not; a file
    /// with a bare CR is refused, CRLF is fine.
    #[test]
    fn bare_cr_is_an_error() {
        let err = |line| ScanError {
            line: Some(line),
            msg: "bare CR line endings are not supported".into(),
        };
        for mask_templates in [false, true] {
            let opts = ScanOpts { mask_templates };
            assert_eq!(scan("a: x\rb: y\r", opts), Err(err(1)));
            assert_eq!(scan("a: x\r\nb: y\rc: z\r\n", opts), Err(err(2)));
            assert_eq!(scan("a: x\n\nb: y\r", opts), Err(err(3)));
            assert_eq!(scan("a: &x # c\r b\n", opts), Err(err(1)));
            assert!(scan("a: x\r\nb: y\r\n", opts).is_ok());
        }
    }

    /// Review M1: the masked bytes are blanks, so a plain scalar ends before
    /// or starts after its `{{…}}`; blanks between them count as touching
    /// (Go renders one scalar). A flow separator ends the scalar.
    #[test]
    fn plain_scalar_next_to_a_mask_is_templated() {
        let templated = |t: &str, path: &str| {
            let nodes = scan(
                t,
                ScanOpts {
                    mask_templates: true,
                },
            )
            .unwrap();
            nodes
                .into_iter()
                .find_map(|n| match n {
                    Node::Scalar(s) if path_str(&s.path) == path => Some((s.value, s.templated)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("no scalar at {path} in {t:?}"))
        };
        let yes = |v: &str| (v.to_string(), true);
        let no = |v: &str| (v.to_string(), false);
        // touching, after / before / blank-separated
        assert_eq!(
            templated("image: nginx:1.25-{{ .Values.variant }}\n", "image"),
            yes("nginx:1.25-")
        );
        assert_eq!(
            templated("version: 1.2.{{ .Values.patch }}\n", "version"),
            yes("1.2.")
        );
        assert_eq!(
            templated("image: {{ .Values.registry }}/nginx:1.25\n", "image"),
            yes("/nginx:1.25")
        );
        assert_eq!(templated("a: x {{ y }}\n", "a"), yes("x"));
        assert_eq!(templated("a: {{ y }} x\n", "a"), yes("x"));
        assert_eq!(templated("a: x\t{{ y }}\n", "a"), yes("x"));
        // in a sequence and in a flow sequence
        assert_eq!(templated("l:\n  - x{{ y }}\n", "l[0]"), yes("x"));
        assert_eq!(templated("l: [x{{ y }}]\n", "l[0]"), yes("x"));
        // not adjacent: a flow separator, a comment, another line
        assert_eq!(templated("{k: x, y: {{ .z }}}\n", "k"), no("x"));
        assert_eq!(templated("[x, {{ .z }}]\n", "[0]"), no("x"));
        assert_eq!(templated("a: x  # {{ c }}\n", "a"), no("x"));
        assert_eq!(templated("a: x\n{{- if .z }}\nb: y\n", "a"), no("x"));
    }

    /// Review M2: a line break inside a scalar token is not a place for a
    /// comment; the end of the token's own line (or of a one-line sibling)
    /// is.
    #[test]
    fn inside_token_finds_line_breaks_in_scalars() {
        let at = |t: &str, i: usize| inside_token(&scan(t, ScanOpts::default()).unwrap(), i);

        // multi-line plain sibling in a flow mapping: the break is inside
        let t = "include: {project: g/p, ref: main, file: a\n  .yml}\n";
        let brk = t.find('\n').unwrap();
        assert!(at(t, brk));
        // … also CRLF (the `\r` of the break), and double-quoted
        let t = "include: {project: g/p, ref: main, file: a\r\n  .yml}\r\n";
        assert!(at(t, t.find('\r').unwrap()));
        let t = "include: {project: g/p, ref: main, file: \"a\\\n  .yml\"}\n";
        assert!(at(t, t.find('\n').unwrap()));
        // a comma at the line end, a one-line value, a block collection
        let t = "include: {project: g/p, ref: main,\n  file: a.yml}\n";
        assert!(!at(t, t.find('\n').unwrap()));
        let t = "a: x\nb: \"y\" # c\n";
        assert!(!at(t, t.find('\n').unwrap()));
        assert!(!at(t, t.len() - 1));
        // token bounds are not inside: start and end of a one-line scalar
        let t = "a: xy\n";
        assert!(!at(t, 3) && !at(t, 5) && at(t, 4));
    }

    /// Characterisation (M8a decides what to report): an unquoted
    /// `{{ … }}` value parses as nested flow mappings without error and
    /// yields no value at the key; masked, it is an implicit null.
    #[test]
    fn unmasked_template_is_not_an_error() {
        let t = "image: {{ .x }}\n";
        let plain = scan(t, ScanOpts::default()).unwrap();
        assert!(!plain.iter().any(|n| matches!(
            n,
            Node::Scalar(s) if path_str(&s.path) == "image" && !s.value.is_empty()
        )));
        let masked = scan(
            t,
            ScanOpts {
                mask_templates: true,
            },
        )
        .unwrap();
        let [Node::Scalar(s)] = &masked[..] else {
            panic!("{masked:?}")
        };
        assert_eq!(path_str(&s.path), "image");
        assert_eq!(s.value, "");
        assert!(s.span.is_empty());
        assert!(!s.templated);
    }

    /// Texts of every test above and below: `scan` returns (no panic) on
    /// every char-boundary prefix, with and without masking, and `Ok` spans
    /// are in bounds on char boundaries.
    const SAMPLES: &[&str] = &[
        "image: alpine:3.20  # c\njob:\n  services:\n    - redis:7\n    - name: node:20\n",
        "a: 1\n---\nb: 2\n...\n---\n- c\n",
        "\u{FEFF}k: x\nä: [y, 🚀z]\nimg: alpine:3.20\n",
        "a: x\r\nb:\r\n  - y\r\n",
        "include: {project: a, ref: b}\ninclude:\n  - project: c\n    ref: d\n",
        "a: 1\nb: c: d\n",
        "image:\n  digest:\n",
        "include: {project: p, ref}\n",
        "include: {ref, project: p}\n",
        "a: &x\n",
        "a: \"x:1\" # c\nb: 'it''s:1'   # c\nc: [\"d:1\" , 'e:1' ]\nd: \"q\\\"q\"  \n",
        "a: &x v1\nb: !!str v2\nc: &y !!str \"v3\"\nd: !!str\n  v4\n",
        "a: &x v\nb: *x\nc:\n  <<: *x\n",
        "a: !reference [.b, c]\nd: !!seq [e]\nf: !custom g\n",
        "a: |\n  x\nb: >-\n  y\nc: |\n  !x\nd: &k >-\n  z\ne: !!str |\n  w\n  w2\n",
        "? [k, l]\n: v\n? *x\n: w\n",
        "a: &p # c\n  v\nb: !t\n  [x]\nc: &a !!str\n\n  \"q\"\n",
        "x: &a\n  - 1\ny: !t &b {k: v}\nz: [&c a, !!str b, *c]\n",
        "a: {{ .x }}\n{{- if y\n }}\nb: \"r:{{ ü }}\"\nc: {{ open\n",
        "img: \"r:{{ .t }}\"\nplain: r:1\n{{- if .x }}\nother: v\n{{- end }}\n",
        "image: {{ .x }}\n",
        "variables:\n  FOO:\tbar\nimage:\tnginx:1.25\n",
        "a:\t- b\n",
        "job:\n\timage: x\n",
    ];

    #[test]
    fn scan_never_panics_on_prefixes() {
        for sample in SAMPLES {
            for end in (0..=sample.len()).filter(|&i| sample.is_char_boundary(i)) {
                let p = &sample[..end];
                for mask_templates in [false, true] {
                    let Ok(nodes) = scan(p, ScanOpts { mask_templates }) else {
                        continue;
                    };
                    let spans = nodes.iter().filter_map(|n| match n {
                        Node::Scalar(s) => Some(&s.span),
                        Node::Alias { span, .. } => Some(span),
                        Node::Tagged { .. } => None,
                    });
                    for sp in spans {
                        assert!(
                            sp.start <= sp.end
                                && sp.end <= p.len()
                                && p.is_char_boundary(sp.start)
                                && p.is_char_boundary(sp.end),
                            "{p:?}: bad span {sp:?}"
                        );
                    }
                }
            }
        }
    }
}
