//! Splice-based text edits: byte-exact replacements and insertions applied
//! in one forward pass, quote-preserving re-quoting, trailing-comment
//! placement and merging.

use crate::text::LineIndex;

/// One edit: replace `start..end` with `text` (`start == end` inserts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Splice {
    pub fn replace(start: usize, end: usize, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }

    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Self::replace(at, at, text)
    }
}

/// Apply non-overlapping splices (offsets into the original `text`) in one
/// forward pass: stable sort by `(start, end)`, so equal-offset insertions
/// keep their given order (R1) and an insertion at a replacement's start
/// goes before it.
pub fn apply(text: &str, s: &[Splice]) -> String {
    let mut splices: Vec<&Splice> = s.iter().collect();
    splices.sort_by_key(|sp| (sp.start, sp.end));
    let mut out =
        String::with_capacity(text.len() + s.iter().map(|sp| sp.text.len()).sum::<usize>());
    let mut cursor = 0;
    for sp in splices {
        debug_assert!(sp.start >= cursor, "overlapping splices: {sp:?}");
        out.push_str(&text[cursor..sp.start]);
        out.push_str(&sp.text);
        cursor = sp.end;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Prefix of a trailing-comment insertion (see [`trailing_comment`]).
const COMMENT: &str = "  # ";

fn is_comment(s: &Splice) -> bool {
    s.start == s.end && s.text.starts_with(COMMENT)
}

/// Merge trailing-comment insertions at the same offset into one
/// `"  # a, b"` at the first one's place, originals in given order (R1:
/// several refs pinned on one line get one comment). Other splices keep
/// their order.
pub fn coalesce_comments(s: Vec<Splice>) -> Vec<Splice> {
    let mut out: Vec<Splice> = Vec::with_capacity(s.len());
    for sp in s {
        if is_comment(&sp)
            && let Some(first) = out
                .iter_mut()
                .find(|o| is_comment(o) && o.start == sp.start)
        {
            first.text.push_str(", ");
            first.text.push_str(&sp.text[COMMENT.len()..]);
            continue;
        }
        out.push(sp);
    }
    out
}

/// Quote-preserving full-value swap — replace `old_token` (as written, quotes
/// optional) with `new_value` (the complete replacement value, unquoted),
/// keeping the token's original quote style.
pub fn requote(old_token: &str, new_value: &str) -> String {
    let b = old_token.as_bytes();
    let quote: Option<char> =
        if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
            Some(b[0] as char)
        } else {
            None
        };
    match quote {
        Some(q) => format!("{q}{new_value}{q}"),
        None => new_value.to_string(),
    }
}

/// The trailing-comment insertion (`"  # {orig}"`) for `line` (0-based) after
/// the token ending at `token_end`: placed at the line end per `lines`, the
/// [`LineIndex`] of `text` built once per file (review L3; before `\n` or
/// `\r\n` — R2, fixed in C12; `text.len()` for a final line without a
/// newline). `None` when a `#` occurs after the token on the same line — a
/// comment is already there — or the line does not exist.
pub fn trailing_comment(
    text: &str,
    lines: &LineIndex,
    line: usize,
    token_end: usize,
    orig: &str,
) -> Option<Splice> {
    let line_end = lines.line_end(line)?;
    if token_end > line_end || text[token_end..line_end].contains('#') {
        return None;
    }
    Some(Splice::insert(line_end, format!("  # {orig}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oci::{ImageRef, splice_value};

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn apply_multiple_spans_end_first() {
        let text = "image: alpine:3.20\nservices: [alpine:3.20, redis:7]\n";
        let spans = [
            text.find("alpine:3.20").unwrap(),
            text.rfind("alpine:3.20").unwrap(),
            text.find("redis:7").unwrap(),
        ];
        let splices: Vec<Splice> = spans
            .iter()
            .map(|&start| {
                let end = start + text[start..].find([',', ']', '\n']).unwrap();
                let token = &text[start..end];
                let r = ImageRef::parse(token).unwrap();
                Splice::replace(start, end, splice_value(token, &r, DIGEST))
            })
            .collect();
        assert_eq!(
            apply(text, &splices),
            format!(
                "image: alpine:3.20@{DIGEST}\nservices: [alpine:3.20@{DIGEST}, redis:7@{DIGEST}]\n"
            )
        );
    }

    #[test]
    fn requote_preserves_quotes() {
        let sha = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
        assert_eq!(requote("\"main\"", sha), format!("\"{sha}\""));
        assert_eq!(requote("'main'", sha), format!("'{sha}'"));
        assert_eq!(requote("main", sha), sha);
    }

    #[test]
    fn apply_inserts_and_replaces() {
        let text = "  ref: main\n";
        // Given in ascending order: the insertion at offset 11 is only
        // correct if applied before the replacement at 7..11 (end-first).
        let splices = vec![
            Splice::replace(7, 11, "abcdef0123456789abcdef0123456789abcdef01"),
            Splice::insert(11, "  # main"),
        ];
        assert_eq!(
            apply(text, &splices),
            "  ref: abcdef0123456789abcdef0123456789abcdef01  # main\n"
        );
    }

    #[test]
    fn apply_equal_start_insertions_keep_source_order() {
        // R1 (fixed in C11): equal-offset insertions keep their given order.
        assert_eq!(
            apply(
                "x\n",
                &[Splice::insert(1, "  # a"), Splice::insert(1, "  # b")]
            ),
            "x  # a  # b\n"
        );
    }

    #[test]
    fn coalesce_merges_same_offset_comments() {
        let merged = coalesce_comments(vec![
            Splice::insert(36, "  # main"),
            Splice::replace(0, 3, "abc"),
            Splice::insert(36, "  # dev"),
            Splice::insert(40, "  # x"),
        ]);
        assert_eq!(
            merged,
            vec![
                Splice::insert(36, "  # main, dev"),
                Splice::replace(0, 3, "abc"),
                Splice::insert(40, "  # x"),
            ]
        );
        // A same-offset insertion that is not a trailing comment stays.
        let other = vec![Splice::insert(36, "  # main"), Splice::insert(36, "!")];
        assert_eq!(coalesce_comments(other.clone()), other);
    }

    #[test]
    fn trailing_comment_uses_the_given_index() {
        let t = "a: x\r\nb: y\n";
        let li = LineIndex::new(t);
        assert_eq!(
            trailing_comment(t, &li, 0, 4, "o"),
            Some(Splice::insert(4, "  # o"))
        );
        assert_eq!(
            trailing_comment(t, &li, 1, 10, "o"),
            Some(Splice::insert(t.len() - 1, "  # o"))
        );
    }

    #[test]
    fn trailing_comment_bare_line() {
        let text = "  ref: main\n";
        assert_eq!(
            trailing_comment(text, &LineIndex::new(text), 0, 11, "main"),
            Some(Splice::insert(11, "  # main"))
        );
    }

    #[test]
    fn trailing_comment_existing_comment() {
        let text = "  ref: main  # keep\n";
        assert_eq!(
            trailing_comment(text, &LineIndex::new(text), 0, 11, "main"),
            None
        );
    }

    #[test]
    fn trailing_comment_before_crlf() {
        let text = "x: a\r\ny: b\r\n";
        assert_eq!(
            trailing_comment(text, &LineIndex::new(text), 1, 10, "b"),
            Some(Splice::insert(10, "  # b"))
        );
        assert_eq!(
            trailing_comment(text, &LineIndex::new(text), 0, 4, "a"),
            Some(Splice::insert(4, "  # a"))
        );
    }

    #[test]
    fn trailing_comment_flow_line() {
        let text = "include: { project: g/p, ref: main }\n";
        assert_eq!(
            trailing_comment(text, &LineIndex::new(text), 0, 34, "main"),
            Some(Splice::insert(36, "  # main"))
        );
    }

    #[test]
    fn apply_equal_value_is_identity() {
        let text = "image: alpine:3.20\n";
        let start = text.find("alpine").unwrap();
        let end = start + "alpine:3.20".len();
        assert_eq!(
            apply(text, &[Splice::replace(start, end, "alpine:3.20")]),
            text
        );
    }

    #[test]
    fn apply_preserves_trailing_comment() {
        let text = "image: alpine:3.20   # base image\nother: keep\n";
        let start = text.find("alpine").unwrap();
        let end = start + "alpine:3.20".len();
        assert_eq!(
            apply(
                text,
                &[Splice::replace(start, end, format!("alpine:3.20@{DIGEST}"))]
            ),
            format!("image: alpine:3.20@{DIGEST}   # base image\nother: keep\n")
        );
    }
}
