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

use std::ops::Range;

use saphyr_parser::{Event, Parser, ScalarStyle, Span};

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
    /// The decoded value.
    pub value: String,
    /// The anchor id when this scalar defines one.
    pub anchor: Option<usize>,
    /// Id of the enclosing collection (ids from 1, in event order); 0 when
    /// the document is this scalar. Groups mapping siblings.
    pub parent: usize,
    /// The enclosing collection is a flow collection (or nested in one).
    pub in_flow: bool,
    /// A non-empty span overlaps a masked `{{…}}`.
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

struct Walker<'a> {
    /// The original text: its char boundaries bound every span.
    text: &'a str,
    /// The text the parser saw (same bytes unless masked).
    src: &'a [u8],
    /// Parser char index → byte offset in `src`.
    c2b: Vec<usize>,
    lines: LineIndex,
    stack: Vec<Frame>,
    next_id: usize,
    docs_seen: usize,
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

    fn push_collection(&mut self, is_map: bool, span: &Span) -> Result<(), ScanError> {
        let (s, e) = self.bytes(span)?;
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

    fn event(&mut self, ev: Event<'_>, span: &Span) -> Result<(), ScanError> {
        match ev {
            Event::DocumentStart(_) => {
                self.docs_seen += 1;
                self.stack.clear();
            }
            Event::Scalar(value, style, anchor, _tag) => {
                if self.at_key() {
                    if let Some(Frame::Map { key, want_key, .. }) = self.stack.last_mut() {
                        *key = Some(PathSeg::Key(value.into_owned()));
                        *want_key = false;
                    }
                    return Ok(());
                }
                let (s, e) = self.bytes(span)?;
                let style = match style {
                    ScalarStyle::Plain => Style::Plain,
                    ScalarStyle::SingleQuoted => Style::Single,
                    ScalarStyle::DoubleQuoted => Style::Double,
                    ScalarStyle::Literal | ScalarStyle::Folded => Style::Block,
                };
                let span = if style == Style::Plain && value.is_empty() {
                    s..s
                } else {
                    s..e
                };
                self.nodes.push(Node::Scalar(ScalarRef {
                    path: self.path(),
                    doc: self.docs_seen.saturating_sub(1),
                    line: self.lines.line_of(span.start),
                    span,
                    style,
                    value: value.into_owned(),
                    anchor: (anchor > 0).then_some(anchor),
                    parent: self.parent(),
                    in_flow: self.in_flow(),
                    templated: false,
                }));
                self.advance();
            }
            Event::MappingStart(..) => self.push_collection(true, span)?,
            Event::SequenceStart(..) => self.push_collection(false, span)?,
            Event::MappingEnd | Event::SequenceEnd => {
                self.stack.pop();
                self.advance();
            }
            _ => {}
        }
        Ok(())
    }
}

/// Every value node of `text` in event order.
///
/// `Err` on the first parser error (sticky upstream): no partial result.
/// The message is `invalid YAML: {parser info}`, `line` the 1-based line of
/// the error.
pub fn scan(text: &str, _opts: ScanOpts) -> Result<Vec<Node>, ScanError> {
    let mut c2b: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
    c2b.push(text.len());
    let mut walker = Walker {
        text,
        src: text.as_bytes(),
        c2b,
        lines: LineIndex::new(text),
        stack: Vec::new(),
        next_id: 1,
        docs_seen: 0,
        nodes: Vec::new(),
    };
    for item in Parser::new_from_str(text) {
        let (ev, span) = item.map_err(|e| ScanError {
            line: Some(e.marker().line()),
            msg: format!("invalid YAML: {}", e.info()),
        })?;
        walker.event(ev, &span)?;
    }
    Ok(walker.nodes)
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
}
