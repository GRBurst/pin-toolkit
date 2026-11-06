//! Byte-exact splicing of discovered image-reference tokens.

use crate::discover::PinTarget;
use crate::image_ref::ImageRef;

/// Splice replacement tokens into `text` for the given targets.
///
/// `new_by_start` maps `PinTarget::start()` to the replacement token
/// (quotes preserved). Returns `(new_text, number of replacements actually
/// applied)`. Replacements whose value equals the current token count as
/// 0. Offsets are applied end-first so earlier offsets stay valid.
pub fn apply(
    text: &str,
    targets: &[PinTarget],
    new_by_start: &std::collections::BTreeMap<usize, String>,
) -> (String, usize) {
    let splices: Vec<(usize, usize, String)> = targets
        .iter()
        .filter_map(|t| {
            let new = new_by_start.get(&t.start())?;
            let old = &text[t.start()..t.end()];
            (new != old).then(|| (t.start(), t.end(), new.clone()))
        })
        .collect();
    apply_splices(text, &splices)
}

/// M3 (Task 4): apply splices `(start, end, new)` — a replacement when
/// `start < end`, an insertion when `start == end` — in **end-first**
/// order (descending `start`, stable for equal starts), so earlier
/// splices' offsets stay valid for later ones. Returns
/// `(new_text, number applied)`.
pub fn apply_splices(text: &str, splices: &[(usize, usize, String)]) -> (String, usize) {
    let mut splices = splices.to_vec();
    splices.sort_by_key(|&(start, _, _)| std::cmp::Reverse(start));
    let applied = splices.len();
    let mut out = text.to_string();
    for (start, end, new) in splices {
        out.replace_range(start..end, &new);
    }
    (out, applied)
}

/// M3 (Task 4): quote-preserving full-value swap — replace `old_token`
/// (as written, quotes optional) with `new_value` (the complete
/// replacement value, unquoted), keeping the token's original quote style.
pub fn splice_replace(old_token: &str, new_value: &str) -> String {
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

/// M3 (Task 4): the byte offset at which a trailing comment may be
/// inserted on `line`, after the token ending at `token_end`: the line end
/// (just before `\n`; `text.len()` for a final line without a newline).
/// `None` when a `#` occurs after the token on the same line — a trailing
/// comment is already there, so none is added.
pub fn comment_insert_offset(text: &str, line: usize, token_end: usize) -> Option<usize> {
    let line_start = text.lines().take(line).map(|l| l.len() + 1).sum::<usize>();
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    if token_end > line_end || text[token_end..line_end].contains('#') {
        return None;
    }
    Some(line_end)
}

/// Build the replacement token for a discovered reference.
///
/// `old_token` is the token as written (quotes optional). If it already
/// ends with `@sha256:<64hex>` (i.e. `ref_.digest` is `Some`), the digest
/// part is replaced; otherwise `@<digest>` is appended to the
/// quote-stripped value and re-quoted with the original style.
pub fn splice_value(old_token: &str, ref_: &ImageRef, digest: &str) -> String {
    let b = old_token.as_bytes();
    let quote: Option<char> =
        if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
            Some(b[0] as char)
        } else {
            None
        };
    let inner: &str = match quote {
        Some(_) => &old_token[1..old_token.len() - 1],
        None => old_token,
    };

    let value = if ref_.digest.is_some() {
        let at = inner
            .rfind('@')
            .expect("digest is set, so a digest separator '@' must be present");
        format!("{}@{}", &inner[..at], digest)
    } else {
        format!("{inner}@{digest}")
    };

    match quote {
        Some(q) => format!("{q}{value}{q}"),
        None => value,
    }
}

/// Tag bump + pin: `alpine:3.20` → `alpine:3.24@sha256:...`. Quote-preserving
/// like `splice_value`. Callers guarantee: ref has a tag and no digest.
pub fn splice_update(old_token: &str, new_tag: &str, digest: &str) -> String {
    let b = old_token.as_bytes();
    let quote: Option<char> =
        if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
            Some(b[0] as char)
        } else {
            None
        };
    let inner: &str = match quote {
        Some(_) => &old_token[1..old_token.len() - 1],
        None => old_token,
    };
    // Repo part = everything before the tag's colon (the last `:` after the
    // last `/`); fall back to cutting at the last `/` when the tag is not
    // separated by a colon.
    let repo_part = match (inner.rfind('/'), inner.rfind(':')) {
        (Some(slash), Some(colon)) if colon > slash => &inner[..colon],
        (Some(slash), _) => &inner[..slash],
        (None, Some(colon)) => &inner[..colon],
        (None, None) => inner,
    };
    let value = format!("{repo_part}:{new_tag}@{digest}");
    match quote {
        Some(q) => format!("{q}{value}{q}"),
        None => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::discover;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn splice_appends_digest_bare() {
        let r = ImageRef::parse("alpine:3.20").unwrap();
        assert_eq!(
            splice_value("alpine:3.20", &r, DIGEST),
            format!("alpine:3.20@{DIGEST}")
        );
    }

    #[test]
    fn splice_appends_digest_tagless() {
        let r = ImageRef::parse("alpine").unwrap();
        assert_eq!(
            splice_value("alpine", &r, DIGEST),
            format!("alpine@{DIGEST}")
        );
    }

    #[test]
    fn splice_preserves_quotes() {
        let r = ImageRef::parse("node:20").unwrap();
        assert_eq!(
            splice_value("\"node:20\"", &r, DIGEST),
            format!("\"node:20@{DIGEST}\"")
        );
    }

    #[test]
    fn splice_replaces_existing_digest() {
        let old = format!("alpine:3.20@{DIGEST}");
        let r = ImageRef::parse(&old).unwrap();
        let other = "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        assert_eq!(
            splice_value(&old, &r, other),
            format!("alpine:3.20@{other}")
        );
    }

    #[test]
    fn splice_update_bumps_and_pins_bare() {
        assert_eq!(
            splice_update("alpine:3.20", "3.24", DIGEST),
            format!("alpine:3.24@{DIGEST}")
        );
    }

    #[test]
    fn splice_update_preserves_quotes() {
        assert_eq!(
            splice_update("\"node:20\"", "22", DIGEST),
            format!("\"node:22@{DIGEST}\"")
        );
    }

    #[test]
    fn splice_update_keeps_repo_path() {
        assert_eq!(
            splice_update("library/alpine:3.20", "3.24", DIGEST),
            format!("library/alpine:3.24@{DIGEST}")
        );
    }

    #[test]
    fn apply_splices_multiple_spans_end_first() {
        let text = "image: alpine:3.20\nservices: [alpine:3.20, redis:7]\n";
        let t = discover(text);
        let mut m = std::collections::BTreeMap::new();
        for x in &t {
            m.insert(
                x.start(),
                splice_value(
                    &text[x.start()..x.end()],
                    &ImageRef::parse(x.value()).unwrap(),
                    DIGEST,
                ),
            );
        }
        let (out, n) = apply(text, &t, &m);
        assert_eq!(n, 3);
        assert_eq!(
            out,
            format!(
                "image: alpine:3.20@{DIGEST}\nservices: [alpine:3.20@{DIGEST}, redis:7@{DIGEST}]\n"
            )
        );
    }

    #[test]
    fn splice_replace_preserves_quotes() {
        let sha = "e3262fdd0914fa823210cdb79a8c421e2cef79d8";
        assert_eq!(splice_replace("\"main\"", sha), format!("\"{sha}\""));
        assert_eq!(splice_replace("'main'", sha), format!("'{sha}'"));
        assert_eq!(splice_replace("main", sha), sha);
    }

    #[test]
    fn apply_splices_inserts_and_replaces() {
        let text = "  ref: main\n";
        // Given in ascending order: the insertion at offset 11 is only
        // correct if applied before the replacement at 7..11 (end-first).
        let splices = vec![
            (
                7,
                11,
                String::from("abcdef0123456789abcdef0123456789abcdef01"),
            ),
            (11, 11, String::from("  # main")),
        ];
        let (out, n) = apply_splices(text, &splices);
        assert_eq!(n, 2);
        assert_eq!(
            out,
            "  ref: abcdef0123456789abcdef0123456789abcdef01  # main\n"
        );
    }

    #[test]
    fn comment_offset_bare_line() {
        let text = "  ref: main\n";
        assert_eq!(comment_insert_offset(text, 0, 11), Some(11));
    }

    #[test]
    fn comment_offset_existing_comment() {
        let text = "  ref: main  # keep\n";
        assert_eq!(comment_insert_offset(text, 0, 11), None);
    }

    #[test]
    fn comment_offset_flow_line() {
        let text = "include: { project: g/p, ref: main }\n";
        assert_eq!(comment_insert_offset(text, 0, 34), Some(36));
    }

    #[test]
    fn apply_is_noop_when_values_equal() {
        let text = "image: alpine:3.20\n";
        let t = discover(text);
        let m = std::collections::BTreeMap::from([(t[0].start(), t[0].value().to_string())]);
        let (out, n) = apply(text, &t, &m);
        assert_eq!(n, 0);
        assert_eq!(out, text);
    }

    #[test]
    fn apply_preserves_trailing_comment() {
        let text = "image: alpine:3.20   # base image\nother: keep\n";
        let t = discover(text);
        let m = std::collections::BTreeMap::from([(t[0].start(), format!("alpine:3.20@{DIGEST}"))]);
        let (out, _) = apply(text, &t, &m);
        assert_eq!(
            out,
            format!("image: alpine:3.20@{DIGEST}   # base image\nother: keep\n")
        );
    }
}
