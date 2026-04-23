//! LSP-compatible positions and byte ranges.

use serde::{Deserialize, Serialize};

/// A location in a source file (LSP-compatible).
///
/// - `line` is 0-indexed
/// - `character` is 0-indexed UTF-16 code units (per LSP spec)
/// - `offset` is the byte offset into the source
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Position {
    pub line: u32,
    pub character: u32,
    pub offset: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Maps byte offsets to (line, utf-16 character) positions for a source string.
///
/// Built once per file, then reused for every lookup.
pub struct LineIndex<'a> {
    source: &'a str,
    /// Byte offset of the start of each line (line 0 starts at 0).
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.source.len());
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts[line];
        let slice = &self.source[line_start..offset];
        let character: u32 = slice.chars().map(|c| c.len_utf16() as u32).sum();
        Position {
            line: line as u32,
            character,
            offset,
        }
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        Range {
            start: self.position(start),
            end: self.position(end),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line() {
        let src = "hello";
        let idx = LineIndex::new(src);
        assert_eq!(
            idx.position(0),
            Position {
                line: 0,
                character: 0,
                offset: 0
            }
        );
        assert_eq!(
            idx.position(5),
            Position {
                line: 0,
                character: 5,
                offset: 5
            }
        );
    }

    #[test]
    fn multi_line() {
        let src = "line0\nline1\nline2";
        let idx = LineIndex::new(src);
        // offset 6 = start of "line1"
        assert_eq!(idx.position(6).line, 1);
        assert_eq!(idx.position(6).character, 0);
        // offset 12 = start of "line2"
        assert_eq!(idx.position(12).line, 2);
    }

    #[test]
    fn utf16_counting_for_surrogate_pair() {
        // "😀" is U+1F600, encoded as 4 bytes in UTF-8 and 2 UTF-16 code units
        let src = "😀x";
        let idx = LineIndex::new(src);
        // offset 4 = after emoji, before "x"
        assert_eq!(idx.position(4).character, 2);
    }
}
