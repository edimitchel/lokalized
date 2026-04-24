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

    /// Inverse of [`position`]: convert a (line, utf-16 character) pair into
    /// a byte offset in the source. Returns `None` if the position is out of
    /// bounds.
    pub fn offset_at(&self, line: u32, character: u32) -> Option<usize> {
        let line_start = *self.line_starts.get(line as usize)?;
        let next_line_start = self
            .line_starts
            .get(line as usize + 1)
            .copied()
            .unwrap_or(self.source.len());
        let line_text = &self.source[line_start..next_line_start];

        let mut utf16: u32 = 0;
        for (i, c) in line_text.char_indices() {
            if utf16 >= character {
                return Some(line_start + i);
            }
            utf16 += c.len_utf16() as u32;
        }
        if utf16 >= character {
            return Some(next_line_start);
        }
        Some(next_line_start)
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

    #[test]
    fn offset_at_roundtrip() {
        let src = "hello\nworld\nfoo";
        let idx = LineIndex::new(src);
        for offset in 0..=src.len() {
            let pos = idx.position(offset);
            let roundtrip = idx.offset_at(pos.line, pos.character).unwrap();
            assert_eq!(roundtrip, offset, "roundtrip mismatch at {offset}");
        }
    }

    #[test]
    fn offset_at_with_emoji() {
        let src = "a😀b";
        let idx = LineIndex::new(src);
        // 'a' = 1 utf16 unit, 😀 = 2, 'b' = 1
        assert_eq!(idx.offset_at(0, 0), Some(0));
        assert_eq!(idx.offset_at(0, 1), Some(1)); // before emoji
        assert_eq!(idx.offset_at(0, 3), Some(5)); // after emoji, at 'b'
    }
}
