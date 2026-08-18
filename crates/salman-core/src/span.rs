//! Source positions.
//!
//! Every diagnostic, AST node and runtime symbol carries a [`Span`] so that an
//! engineer can be shown the exact bytes that caused a problem. Spans are byte
//! offsets, not line/column pairs: line and column are derived on demand, which
//! keeps the hot paths cheap and avoids storing a position model that has to be
//! kept in step with edits.

use std::fmt;

/// Largest source file salman will load, in bytes.
///
/// Source text arrives from files salman did not write. A hard ceiling means a
/// hostile or corrupt file cannot make the loader allocate without bound, and
/// it lets offsets stay `u32`. 64 MiB is roughly two orders of magnitude larger
/// than the largest real PLC source file the author has seen.
pub const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;

/// Identifies one source file within a [`SourceMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(u32);

impl FileId {
    /// The numeric index of this file within its source map.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// A half-open byte range `[start, end)` within a single source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    /// The file this span refers to.
    pub file: FileId,
    /// Inclusive start byte offset.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

impl Span {
    /// Creates a span. `start` and `end` are byte offsets; `end` is clamped to
    /// be at least `start` so a span is never inverted.
    #[must_use]
    pub const fn new(file: FileId, start: u32, end: u32) -> Self {
        Self {
            file,
            start,
            end: if end < start { start } else { end },
        }
    }

    /// A zero-width span at `offset`, used to point between two characters
    /// (for example "expected `;` here").
    #[must_use]
    pub const fn at(file: FileId, offset: u32) -> Self {
        Self {
            file,
            start: offset,
            end: offset,
        }
    }

    /// Length in bytes.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the span is zero-width.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The smallest span covering both inputs.
    ///
    /// Returns `self` unchanged if the spans are in different files, because a
    /// span that straddles two files cannot be rendered.
    #[must_use]
    pub fn join(self, other: Self) -> Self {
        if self.file != other.file {
            return self;
        }
        Self {
            file: self.file,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

/// A one-based line and column, for display only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineCol {
    /// One-based line number.
    pub line: u32,
    /// One-based column, counted in Unicode scalar values, not bytes.
    pub column: u32,
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// One loaded source file.
#[derive(Debug, Clone)]
pub struct SourceFile {
    id: FileId,
    name: String,
    text: String,
    /// Byte offset of the first character of each line. Always starts with 0.
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// The file's identifier within its source map.
    #[must_use]
    pub const fn id(&self) -> FileId {
        self.id
    }

    /// The display name, normally a path relative to the project root.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The full text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Number of lines. A file with no trailing newline still has its last
    /// partial line counted.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        self.line_starts.len() as u32
    }

    /// Converts a byte offset to a one-based line and column.
    ///
    /// Offsets past the end of the file are clamped to the end, so this never
    /// fails and never panics — it is called from diagnostic rendering, which
    /// must work even when a parser reports a slightly wrong offset.
    #[must_use]
    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = offset.min(self.text.len() as u32);
        // partition_point gives the number of line starts <= offset, which is
        // the one-based line number.
        let line_index = self
            .line_starts
            .partition_point(|&s| s <= offset)
            .saturating_sub(1);
        let line_start = self.line_starts.get(line_index).copied().unwrap_or(0);
        let prefix = self
            .text
            .get(line_start as usize..offset as usize)
            .unwrap_or("");
        LineCol {
            line: line_index as u32 + 1,
            column: prefix.chars().count() as u32 + 1,
        }
    }

    /// The text of a one-based line, without its terminator.
    #[must_use]
    pub fn line_text(&self, line: u32) -> &str {
        let Some(index) = (line as usize).checked_sub(1) else {
            return "";
        };
        let Some(&start) = self.line_starts.get(index) else {
            return "";
        };
        let end = self
            .line_starts
            .get(index + 1)
            .copied()
            .unwrap_or(self.text.len() as u32);
        self.text
            .get(start as usize..end as usize)
            .unwrap_or("")
            .trim_end_matches('\n')
            .trim_end_matches('\r')
    }

    /// The source text a span covers, or `""` if the span is out of range or
    /// does not fall on character boundaries.
    #[must_use]
    pub fn span_text(&self, span: Span) -> &str {
        if span.file != self.id {
            return "";
        }
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }
}

/// Why a source file could not be added to a [`SourceMap`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The file is larger than [`MAX_SOURCE_BYTES`].
    TooLarge {
        /// Display name of the rejected file.
        name: String,
        /// Its size in bytes.
        bytes: usize,
    },
    /// More than `u32::MAX` files were added.
    TooManyFiles,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { name, bytes } => write!(
                f,
                "{name} is {bytes} bytes, which exceeds salman's {MAX_SOURCE_BYTES} byte source limit"
            ),
            Self::TooManyFiles => f.write_str("too many source files"),
        }
    }
}

impl std::error::Error for SourceError {}

/// The set of source files a compilation knows about.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    /// An empty map.
    #[must_use]
    pub const fn new() -> Self {
        Self { files: Vec::new() }
    }

    /// Adds a file and returns its identifier.
    ///
    /// # Errors
    ///
    /// Returns [`SourceError::TooLarge`] if the text exceeds
    /// [`MAX_SOURCE_BYTES`], or [`SourceError::TooManyFiles`] if the map is
    /// full.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<FileId, SourceError> {
        let name = name.into();
        let text = text.into();
        if text.len() > MAX_SOURCE_BYTES {
            return Err(SourceError::TooLarge {
                name,
                bytes: text.len(),
            });
        }
        let Ok(index) = u32::try_from(self.files.len()) else {
            return Err(SourceError::TooManyFiles);
        };
        let id = FileId(index);
        let line_starts = compute_line_starts(&text);
        self.files.push(SourceFile {
            id,
            name,
            text,
            line_starts,
        });
        Ok(id)
    }

    /// Looks a file up by id.
    #[must_use]
    pub fn file(&self, id: FileId) -> Option<&SourceFile> {
        self.files.get(id.index())
    }

    /// Every file, in the order they were added.
    pub fn files(&self) -> impl Iterator<Item = &SourceFile> {
        self.files.iter()
    }

    /// Number of files in the map.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Whether the map holds no files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// Byte offsets of the first character of every line.
///
/// A `\r\n` pair terminates one line; a lone `\r` does not, because IEC source
/// files in the wild are LF or CRLF and treating a bare `\r` as a terminator
/// makes column numbers wrong in files that contain one inside a string.
fn compute_line_starts(text: &str) -> Vec<u32> {
    let mut starts = vec![0u32];
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(offset as u32 + 1);
        }
    }
    starts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map_with(text: &str) -> (SourceMap, FileId) {
        let mut map = SourceMap::new();
        let id = map.add("test.st", text).unwrap();
        (map, id)
    }

    #[test]
    fn line_col_is_one_based() {
        let (map, id) = map_with("abc\ndef\n");
        let f = map.file(id).unwrap();
        assert_eq!(f.line_col(0), LineCol { line: 1, column: 1 });
        assert_eq!(f.line_col(2), LineCol { line: 1, column: 3 });
        assert_eq!(f.line_col(4), LineCol { line: 2, column: 1 });
    }

    #[test]
    fn line_col_counts_columns_in_characters_not_bytes() {
        // A comment may legally contain non-ASCII text. If columns were byte
        // offsets the caret in a rendered diagnostic would land in the wrong
        // place for anyone writing in a language with accents.
        let (map, id) = map_with("(* ä *)\nX := 1;");
        let f = map.file(id).unwrap();
        let offset = "(* ä ".len() as u32;
        assert_eq!(f.line_col(offset), LineCol { line: 1, column: 6 });
    }

    #[test]
    fn line_col_clamps_offsets_past_the_end_instead_of_panicking() {
        let (map, id) = map_with("abc");
        let f = map.file(id).unwrap();
        assert_eq!(f.line_col(9_999), LineCol { line: 1, column: 4 });
    }

    #[test]
    fn line_col_on_an_offset_inside_a_character_does_not_panic() {
        let (map, id) = map_with("ä");
        let f = map.file(id).unwrap();
        // Offset 1 is the middle of the two-byte 'ä'.
        let _ = f.line_col(1);
    }

    #[test]
    fn crlf_line_endings_produce_the_same_line_numbers_as_lf() {
        let (lf_map, lf) = map_with("a\nb\nc");
        let (crlf_map, crlf) = map_with("a\r\nb\r\nc");
        for (offset_lf, offset_crlf) in [(0u32, 0u32), (2, 3), (4, 6)] {
            assert_eq!(
                lf_map.file(lf).unwrap().line_col(offset_lf).line,
                crlf_map.file(crlf).unwrap().line_col(offset_crlf).line
            );
        }
    }

    #[test]
    fn line_text_strips_the_terminator_for_lf_and_crlf() {
        let (map, id) = map_with("alpha\r\nbeta\n");
        let f = map.file(id).unwrap();
        assert_eq!(f.line_text(1), "alpha");
        assert_eq!(f.line_text(2), "beta");
        assert_eq!(f.line_text(99), "");
        assert_eq!(f.line_text(0), "");
    }

    #[test]
    fn span_join_takes_the_outer_bounds() {
        let file = FileId(0);
        let a = Span::new(file, 2, 5);
        let b = Span::new(file, 10, 12);
        assert_eq!(a.join(b), Span::new(file, 2, 12));
        assert_eq!(b.join(a), Span::new(file, 2, 12));
    }

    #[test]
    fn span_is_never_inverted() {
        let s = Span::new(FileId(0), 10, 3);
        assert_eq!(s.start, 10);
        assert_eq!(s.end, 10);
        assert!(s.is_empty());
    }

    #[test]
    fn oversized_sources_are_rejected_rather_than_loaded() {
        let mut map = SourceMap::new();
        let big = "x".repeat(MAX_SOURCE_BYTES + 1);
        let err = map.add("huge.st", big).unwrap_err();
        assert!(matches!(err, SourceError::TooLarge { .. }));
        assert!(map.is_empty());
    }

    #[test]
    fn span_text_of_a_foreign_file_is_empty_rather_than_wrong() {
        let (map, id) = map_with("hello");
        let f = map.file(id).unwrap();
        let foreign = Span::new(FileId(7), 0, 3);
        assert_eq!(f.span_text(foreign), "");
    }

    #[test]
    fn empty_file_has_one_line() {
        let (map, id) = map_with("");
        let f = map.file(id).unwrap();
        assert_eq!(f.line_count(), 1);
        assert_eq!(f.line_col(0), LineCol { line: 1, column: 1 });
    }
}
