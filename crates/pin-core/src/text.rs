//! Line model over a text's bytes.

/// Byte offsets of every line (0-based): `starts[l]` is the first byte,
/// `ends[l]` the end of the content — excluding the `"\n"` and one `"\r"`
/// before it, so CRLF lines end before `\r\n`. A final line without a
/// newline ends at `text.len()`.
///
/// Shared line model for edits and (M5c-y) `core::yaml`; `discover.rs`
/// keeps its own split until M5c-y.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    starts: Vec<usize>,
    ends: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let (mut starts, mut ends) = (Vec::new(), Vec::new());
        let mut pos = 0;
        for l in text.split_inclusive('\n') {
            let body = l.strip_suffix('\n').unwrap_or(l);
            let body = body.strip_suffix('\r').unwrap_or(body);
            starts.push(pos);
            ends.push(pos + body.len());
            pos += l.len();
        }
        Self { starts, ends }
    }

    /// Number of lines (`""` has none; `"a\n"` has one).
    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    pub fn line_start(&self, line: usize) -> Option<usize> {
        self.starts.get(line).copied()
    }

    pub fn line_end(&self, line: usize) -> Option<usize> {
        self.ends.get(line).copied()
    }

    /// 0-based line containing `byte`; past the last line start → the last
    /// line; 0 for `""`.
    pub fn line_of(&self, byte: usize) -> usize {
        self.starts
            .partition_point(|&s| s <= byte)
            .saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_lf_crlf_and_eof() {
        let ix = LineIndex::new("a\r\nbb\nc");
        assert_eq!(ix.len(), 3);
        assert!(!ix.is_empty());
        let starts: Vec<_> = (0..3).map(|l| ix.line_start(l).unwrap()).collect();
        let ends: Vec<_> = (0..3).map(|l| ix.line_end(l).unwrap()).collect();
        assert_eq!(starts, [0, 3, 6]);
        assert_eq!(ends, [1, 5, 7]);
        assert_eq!(ix.line_start(3), None);
        assert_eq!(ix.line_end(3), None);
        assert!(LineIndex::new("").is_empty());
    }

    #[test]
    fn line_of_maps_bytes_to_lines() {
        let ix = LineIndex::new("a\r\nbb\nc");
        let lines: Vec<_> = [0, 2, 3, 6, 7].iter().map(|&b| ix.line_of(b)).collect();
        assert_eq!(lines, [0, 0, 1, 2, 2]);
        assert_eq!(LineIndex::new("").line_of(0), 0);
    }
}
