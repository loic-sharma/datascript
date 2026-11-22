//! Source text with line indexes and position conversions.
//!
//! Three ways of counting positions exist, each used by one kind of consumer:
//!
//! - **bytes** (UTF-8): [`Span`] offsets, used everywhere inside the compiler;
//! - **characters**: 1-based columns in the terminal and plain renderers;
//! - **UTF-16 code units**: offsets and columns in the JSON form and the LSP (D-025).
//!
//! A line ends at `\n`, `\r\n` or a lone `\r`, as in the LSP and as the parser counts lines. The
//! terminator is never shown.

use crate::span::{FileId, Span};

/// A 1-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineCol {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column. Its unit depends on the method that produced it.
    pub column: u32,
}

/// One source file: its display name, text and line index.
#[derive(Debug, Clone)]
pub struct SourceFile {
    name: String,
    text: String,
    /// Byte offset of the start of each line. Always starts with 0.
    line_starts: Vec<u32>,
    /// UTF-16 offset of the start of each line, parallel to `line_starts`.
    line_starts_utf16: Vec<u32>,
}

impl SourceFile {
    /// Creates a source file and indexes its lines.
    ///
    /// # Panics
    ///
    /// If the text is 4 GiB or longer, since spans use 32-bit offsets.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        assert!(
            u32::try_from(text.len()).is_ok(),
            "source files must be smaller than 4 GiB"
        );
        let mut line_starts = vec![0];
        let mut line_starts_utf16 = vec![0];
        let mut utf16 = 0u32;
        let bytes = text.as_bytes();
        for (i, c) in text.char_indices() {
            utf16 += c.len_utf16() as u32;
            // `\r\n` is one terminator: the line starts after its `\n`.
            let ends_line = c == '\n' || (c == '\r' && bytes.get(i + 1) != Some(&b'\n'));
            if ends_line {
                line_starts.push(i as u32 + 1);
                line_starts_utf16.push(utf16);
            }
        }
        SourceFile {
            name: name.into(),
            text,
            line_starts,
            line_starts_utf16,
        }
    }

    /// The display name, e.g. `query.ds`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The full text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines. A trailing `\n` starts a final empty line.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Clamps `offset` to the text and moves it back to the nearest character boundary.
    fn clamp(&self, offset: u32) -> usize {
        let mut offset = (offset as usize).min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    /// The 0-based index of the line containing byte `offset`.
    pub fn line_index(&self, offset: u32) -> usize {
        let offset = self.clamp(offset) as u32;
        self.line_starts.partition_point(|&start| start <= offset) - 1
    }

    /// The text of the 0-based line `index`, without its line terminator.
    ///
    /// Returns an empty string for an index past the end.
    pub fn line_text(&self, index: usize) -> &str {
        let Some(&start) = self.line_starts.get(index) else {
            return "";
        };
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next as usize);
        let line = &self.text[start as usize..end];
        let line = line.strip_suffix('\n').unwrap_or(line);
        line.strip_suffix('\r').unwrap_or(line)
    }

    /// Byte offset where the 0-based line `index` starts.
    pub fn line_start(&self, index: usize) -> u32 {
        self.line_starts
            .get(index)
            .copied()
            .unwrap_or(self.text.len() as u32)
    }

    /// Line and column of byte `offset`, with the column counted in characters.
    ///
    /// This is what the terminal and plain renderers show. A tab counts as one character.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = self.clamp(offset);
        let index = self.line_index(offset as u32);
        let start = self.line_starts[index] as usize;
        let column = self.text[start..offset].chars().count() as u32 + 1;
        LineCol {
            line: index as u32 + 1,
            column,
        }
    }

    /// The UTF-16 offset of byte `offset` from the start of the file.
    pub fn utf16_offset(&self, offset: u32) -> u32 {
        let offset = self.clamp(offset);
        let index = self.line_index(offset as u32);
        let start = self.line_starts[index] as usize;
        self.line_starts_utf16[index] + utf16_len(&self.text[start..offset])
    }

    /// Line and column of byte `offset`, with the column counted in UTF-16 code units.
    ///
    /// This is what the JSON form and the LSP use (D-025).
    pub fn utf16_line_col(&self, offset: u32) -> LineCol {
        let offset = self.clamp(offset);
        let index = self.line_index(offset as u32);
        let start = self.line_starts[index] as usize;
        LineCol {
            line: index as u32 + 1,
            column: utf16_len(&self.text[start..offset]) + 1,
        }
    }

    /// Converts a UTF-16 offset from the start of the file to a byte offset.
    ///
    /// Returns `None` if the offset is past the end or falls inside a surrogate pair.
    pub fn byte_offset_from_utf16(&self, utf16_offset: u32) -> Option<u32> {
        let index = self
            .line_starts_utf16
            .partition_point(|&start| start <= utf16_offset)
            - 1;
        let column = utf16_offset - self.line_starts_utf16[index];
        self.byte_offset_in_line(index, column)
    }

    /// Converts a 1-based line and UTF-16 column (as in [`utf16_line_col`](Self::utf16_line_col))
    /// to a byte offset.
    ///
    /// Returns `None` if the position is outside the file or inside a surrogate pair. A column
    /// may point just past the last character of its line.
    pub fn byte_offset_from_utf16_line_col(&self, position: LineCol) -> Option<u32> {
        let index = (position.line as usize).checked_sub(1)?;
        let column = position.column.checked_sub(1)?;
        if index >= self.line_starts.len() {
            return None;
        }
        self.byte_offset_in_line(index, column)
    }

    /// Byte offset of the 0-based UTF-16 `column` on the 0-based line `index`.
    fn byte_offset_in_line(&self, index: usize, column: u32) -> Option<u32> {
        let start = self.line_starts[index] as usize;
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |&next| next as usize);
        let mut units = 0u32;
        for (i, c) in self.text[start..end].char_indices() {
            if units == column {
                return Some((start + i) as u32);
            }
            units += c.len_utf16() as u32;
            if units > column {
                return None;
            }
        }
        (units == column).then_some(end as u32)
    }
}

fn utf16_len(text: &str) -> u32 {
    text.chars().map(|c| c.len_utf16() as u32).sum()
}

/// All source files of one compilation, addressed by [`FileId`].
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// An empty source map.
    pub fn new() -> Self {
        SourceMap::default()
    }

    /// Adds a file and returns its id.
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        let id = FileId(u32::try_from(self.files.len()).expect("too many source files"));
        self.files.push(SourceFile::new(name, text));
        id
    }

    /// The file with this id, if it exists.
    pub fn get(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    /// The first file with this display name.
    pub fn find(&self, name: &str) -> Option<FileId> {
        self.files
            .iter()
            .position(|file| file.name == name)
            .map(|index| FileId(index as u32))
    }

    /// The text a span covers, if its file exists and the range is valid.
    pub fn snippet(&self, span: Span) -> Option<&str> {
        self.get(span.file)?.text.get(span.range())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lc(line: u32, column: u32) -> LineCol {
        LineCol { line, column }
    }

    #[test]
    fn lines_and_columns() {
        let file = SourceFile::new("q", "ab\ncd\n");
        assert_eq!(file.line_count(), 3);
        assert_eq!(file.line_col(0), lc(1, 1));
        assert_eq!(file.line_col(2), lc(1, 3)); // the `\n` itself
        assert_eq!(file.line_col(3), lc(2, 1));
        assert_eq!(file.line_col(6), lc(3, 1)); // EOF after a trailing newline
        assert_eq!(file.line_text(1), "cd");
        assert_eq!(file.line_text(2), "");
        assert_eq!(file.line_text(9), "");
    }

    #[test]
    fn eof_without_trailing_newline() {
        let file = SourceFile::new("q", "abc");
        assert_eq!(file.line_col(3), lc(1, 4));
        assert_eq!(file.line_col(99), lc(1, 4)); // clamped
        assert_eq!(file.utf16_line_col(3), lc(1, 4));
    }

    #[test]
    fn crlf_is_not_shown() {
        let file = SourceFile::new("q", "ab\r\ncd");
        assert_eq!(file.line_text(0), "ab");
        assert_eq!(file.line_col(4), lc(2, 1));
    }

    #[test]
    fn lone_cr_ends_a_line() {
        // The LSP and the parser treat a lone `\r` as a line break, so positions agree.
        let file = SourceFile::new("q", "ab\rcd\r\nef\n\rg");
        assert_eq!(file.line_count(), 5);
        assert_eq!(file.line_text(0), "ab");
        assert_eq!(file.line_text(1), "cd");
        assert_eq!(file.line_text(2), "ef");
        assert_eq!(file.line_text(3), "");
        assert_eq!(file.line_text(4), "g");
        assert_eq!(file.line_col(3), lc(2, 1));
        assert_eq!(file.utf16_line_col(11), lc(5, 1));
        assert_eq!(file.byte_offset_from_utf16_line_col(lc(2, 3)), Some(5));
    }

    #[test]
    fn tabs_count_as_one_character() {
        let file = SourceFile::new("q", "\t\tx");
        assert_eq!(file.line_col(2), lc(1, 3));
        assert_eq!(file.utf16_line_col(2), lc(1, 3));
    }

    #[test]
    fn non_ascii_columns() {
        // `é` is 2 bytes / 1 UTF-16 unit, `😀` is 4 bytes / 2 UTF-16 units.
        let text = "é😀x";
        let file = SourceFile::new("q", text);
        let x = text.find('x').unwrap() as u32;
        assert_eq!(x, 6);
        assert_eq!(file.line_col(x), lc(1, 3)); // characters
        assert_eq!(file.utf16_line_col(x), lc(1, 4)); // UTF-16
        assert_eq!(file.utf16_offset(x), 3);
        assert_eq!(file.byte_offset_from_utf16(3), Some(x));
        assert_eq!(file.byte_offset_from_utf16(2), None); // inside the surrogate pair
        assert_eq!(file.byte_offset_from_utf16_line_col(lc(1, 4)), Some(x));
    }

    #[test]
    fn offsets_inside_a_character_are_clamped() {
        let file = SourceFile::new("q", "é");
        assert_eq!(file.line_col(1), lc(1, 1));
        assert_eq!(file.utf16_offset(1), 0);
    }

    #[test]
    fn utf16_offsets_on_later_lines() {
        let text = "😀\nab😀c";
        let file = SourceFile::new("q", text);
        let c = text.rfind('c').unwrap() as u32;
        assert_eq!(file.utf16_offset(c), 7); // 2 + 1 (\n) + 2 + 2
        assert_eq!(file.utf16_line_col(c), lc(2, 5));
        assert_eq!(file.byte_offset_from_utf16(7), Some(c));
        assert_eq!(file.byte_offset_from_utf16_line_col(lc(2, 5)), Some(c));
        assert_eq!(
            file.byte_offset_from_utf16_line_col(lc(2, 6)),
            Some(text.len() as u32)
        );
        assert_eq!(file.byte_offset_from_utf16_line_col(lc(2, 7)), None);
        assert_eq!(file.byte_offset_from_utf16_line_col(lc(3, 1)), None);
    }

    #[test]
    fn source_map_lookup() {
        let mut map = SourceMap::new();
        let a = map.add("a.ds", "hello");
        let b = map.add("b.ds", "world");
        assert_eq!(map.find("b.ds"), Some(b));
        assert_eq!(map.get(a).unwrap().name(), "a.ds");
        assert_eq!(map.snippet(Span::new(b, 1, 3)), Some("or"));
        assert_eq!(map.get(FileId(7)).map(SourceFile::name), None);
    }
}
