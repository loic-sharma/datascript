//! Source locations.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// Identifies a [`SourceFile`](crate::SourceFile) inside a [`SourceMap`](crate::SourceMap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FileId(pub u32);

/// A byte range in one source file.
///
/// `start` and `end` are UTF-8 byte offsets with `start <= end`. An empty span (`start == end`)
/// points between two characters, for example at the end of the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Span {
    /// The file the span points into.
    pub file: FileId,
    /// Start byte offset, inclusive.
    pub start: u32,
    /// End byte offset, exclusive.
    pub end: u32,
}

impl Span {
    /// Creates a span. Swaps `start` and `end` if they are given in the wrong order.
    pub fn new(file: FileId, start: u32, end: u32) -> Self {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        Span { file, start, end }
    }

    /// Creates an empty span at `offset`.
    pub fn empty(file: FileId, offset: u32) -> Self {
        Span::new(file, offset, offset)
    }

    /// Length in bytes.
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers no bytes.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// The byte range, for slicing the source text.
    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The smallest span covering both `self` and `other`. Both must be in the same file.
    pub fn to(self, other: Span) -> Span {
        debug_assert_eq!(
            self.file, other.file,
            "cannot join spans from different files"
        );
        Span::new(
            self.file,
            self.start.min(other.start),
            self.end.max(other.end),
        )
    }

    /// Whether the two spans share at least one byte. Empty spans never overlap anything.
    pub fn overlaps(&self, other: &Span) -> bool {
        self.file == other.file && self.start < other.end && other.start < self.end
    }
}
