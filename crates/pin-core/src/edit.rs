//! Splice-based text edits: byte-exact replacements and insertions applied
//! end-first, quote-preserving re-quoting and trailing-comment placement.

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

/// Apply splices in **end-first** order (descending `start`, stable for equal
/// starts), so earlier splices' offsets stay valid for later ones.
pub fn apply(text: &str, s: &[Splice]) -> String {
    let mut splices = s.to_vec();
    splices.sort_by_key(|sp| std::cmp::Reverse(sp.start));
    let mut out = text.to_string();
    for sp in splices {
        out.replace_range(sp.start..sp.end, &sp.text);
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

/// The trailing-comment insertion (`"  # {orig}"`) for `line` after the token
/// ending at `token_end`: placed at the line end (just before `\n`;
/// `text.len()` for a final line without a newline). `None` when a `#` occurs
/// after the token on the same line — a comment is already there.
///
/// R2: the line offset sums `text.lines()` lengths + 1, so CRLF files drift;
/// fixed in C12.
pub fn trailing_comment(text: &str, line: usize, token_end: usize, orig: &str) -> Option<Splice> {
    let line_start = text.lines().take(line).map(|l| l.len() + 1).sum::<usize>();
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
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
    fn apply_equal_start_insertions_reverse_today() {
        // R1 characterised: equal-offset insertions come out in reverse
        // order. Flipped (and fixed) in C11.
        assert_eq!(
            apply(
                "x\n",
                &[Splice::insert(1, "  # a"), Splice::insert(1, "  # b")]
            ),
            "x  # b  # a\n"
        );
    }

    #[test]
    fn trailing_comment_bare_line() {
        let text = "  ref: main\n";
        assert_eq!(
            trailing_comment(text, 0, 11, "main"),
            Some(Splice::insert(11, "  # main"))
        );
    }

    #[test]
    fn trailing_comment_existing_comment() {
        let text = "  ref: main  # keep\n";
        assert_eq!(trailing_comment(text, 0, 11, "main"), None);
    }

    #[test]
    fn trailing_comment_flow_line() {
        let text = "include: { project: g/p, ref: main }\n";
        assert_eq!(
            trailing_comment(text, 0, 34, "main"),
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
