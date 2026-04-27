use lsp_types::Position;

/// Pre-computed line index for efficient position lookups.
///
/// Stores the byte offset of the start of each line, enabling O(1)
/// line-start lookup and O(LineLength) column resolution.
#[derive(Debug, Clone)]
pub struct LineIndex {
    /// Byte offset of the start of each line. `line_offsets[0]` is always 0.
    line_offsets: Vec<usize>,
}

/// Iterator that yields `(byte_offset, &str)` for each Racket character in a string.
/// Racket treats CRLF (`\r\n`) as a single character.
pub struct RacketCharIndices<'a> {
    text: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
}

impl<'a> RacketCharIndices<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            chars: text.char_indices().peekable(),
        }
    }
}

impl<'a> Iterator for RacketCharIndices<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        let (idx, c) = self.chars.next()?;
        let mut end_idx = idx + c.len_utf8();

        if c == '\r' && self.chars.peek().map(|&(_, next_c)| next_c) == Some('\n') {
            let (next_idx, next_c) = self.chars.next().unwrap();
            end_idx = next_idx + next_c.len_utf8();
        }
        
        Some((idx, &self.text[idx..end_idx]))
    }
}

/// Specifies which unit system a column offset uses.
pub enum OffsetUnit {
    /// UTF-16 code units (LSP protocol standard).
    Utf16,
    /// Unicode code points (Racket's `syntax-column`).
    CodePoint,
}

impl LineIndex {
    /// Build a line index from the full document text.
    ///
    /// Handles both `\n` and `\r\n` line endings correctly.
    pub fn new(text: &str) -> Self {
        let mut line_offsets = vec![0usize];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_offsets.push(i + 1);
            }
        }
        Self { line_offsets }
    }

    /// Access the pre-computed line offsets.
    pub fn line_offsets(&self) -> &[usize] {
        &self.line_offsets
    }

    /// Convert a line number and column offset into a byte index into `text`.
    ///
    /// `line` is 0-indexed. `col` is 0-indexed in the specified `unit`.
    pub fn byte_offset(&self, text: &str, line: usize, col: usize, unit: OffsetUnit) -> usize {
        let line_start = self.line_start(line);
        let line_text = &text[line_start..];

        let mut col_remaining = col;
        let chars = RacketCharIndices::new(line_text);
        for (byte_idx, s) in chars {
            if matches!(s, "\n" | "\r" | "\r\n") || col_remaining == 0 {
                return line_start + byte_idx;
            }
            let unit_width = match unit {
                OffsetUnit::Utf16 => s.chars().map(|c| c.len_utf16()).sum(),
                OffsetUnit::CodePoint => 1,
            };
            col_remaining = col_remaining.saturating_sub(unit_width);
        }
        // If we exhaust the line, return end of line text
        line_start + line_text.len()
    }

    /// Convert a Racket code-point column to an LSP UTF-16 column.
    ///
    /// `line` is 0-indexed. `code_point_col` is 0-indexed.
    pub fn code_point_to_utf16(&self, text: &str, line: usize, code_point_col: usize) -> u32 {
        let line_start = self.line_start(line);
        let line_text = &text[line_start..];

        RacketCharIndices::new(line_text)
            .take_while(|(_, s)| !matches!(*s, "\n" | "\r" | "\r\n"))
            .take(code_point_col)
            .map(|(_, s)| s.chars().map(|c| c.len_utf16() as u32).sum::<u32>())
            .sum()
    }

    /// Convert an LSP `Position` (0-indexed line, UTF-16 column) to a byte offset.
    pub fn lsp_position_to_byte(&self, text: &str, pos: Position) -> usize {
        self.byte_offset(text, pos.line as usize, pos.character as usize, OffsetUnit::Utf16)
    }

    /// Convert an absolute byte offset back into a human-readable LSP `Position`.
    pub fn offset_to_position(&self, text: &str, offset: usize) -> Position {
        let line = match self.line_offsets.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        };
        
        let line_start = self.line_offsets[line];
        let col_text = &text[line_start..offset.min(text.len())];
        
        // Calculate UTF-16 character count for the column
        let character = col_text.chars().map(|c| c.len_utf16()).sum::<usize>() as u32;
        
        Position::new(line as u32, character)
    }

    /// Convert a Racket code-point position (1-indexed line, 0-indexed column) and span
    /// into an LSP `Range` (0-indexed line/column, UTF-16).
    pub fn range_from_span(&self, text: &str, line: u32, col: u32, span: u32) -> lsp_types::Range {
        let start_line = line.saturating_sub(1) as usize;
        let start_col = col as usize;
        
        let start_offset = self.byte_offset(text, start_line, start_col, OffsetUnit::CodePoint);
        
        // Find end offset by walking `span` code points
        let end_offset = start_offset
            + RacketCharIndices::new(&text[start_offset..])
                .take(span as usize)
                .map(|(_, s)| s.len())
                .sum::<usize>();
        
        let start_pos = self.offset_to_position(text, start_offset);
        let end_pos = self.offset_to_position(text, end_offset);
        
        lsp_types::Range::new(start_pos, end_pos)
    }

    pub fn get_text_range<'a>(&self, text: &'a str, range: lsp_types::Range) -> &'a str {
        let start = self.lsp_position_to_byte(text, range.start);
        let end = self.lsp_position_to_byte(text, range.end);
        &text[start.min(text.len())..end.min(text.len())]
    }

    fn line_start(&self, line: usize) -> usize {
        self.line_offsets.get(line).copied().unwrap_or_else(|| {
            // Beyond last line — return end of text
            *self.line_offsets.last().unwrap_or(&0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascii_lf() {
        let text = "hello\nworld\n";
        let idx = LineIndex::new(text);
        assert_eq!(idx.line_offsets, vec![0, 6, 12]);
        assert_eq!(idx.byte_offset(text, 0, 0, OffsetUnit::Utf16), 0);
        assert_eq!(idx.byte_offset(text, 0, 5, OffsetUnit::Utf16), 5);
        assert_eq!(idx.byte_offset(text, 1, 0, OffsetUnit::Utf16), 6);
        assert_eq!(idx.byte_offset(text, 1, 3, OffsetUnit::Utf16), 9);
    }

    #[test]
    fn test_crlf() {
        let text = "hello\r\nworld\r\n";
        let idx = LineIndex::new(text);
        // \r\n — \n is at byte 6, so line 1 starts at byte 7
        assert_eq!(idx.line_offsets, vec![0, 7, 14]);
        assert_eq!(idx.byte_offset(text, 0, 0, OffsetUnit::Utf16), 0);
        assert_eq!(idx.byte_offset(text, 0, 5, OffsetUnit::Utf16), 5);
        // Column beyond line content should stop at \r
        assert_eq!(idx.byte_offset(text, 0, 99, OffsetUnit::Utf16), 5);
        assert_eq!(idx.byte_offset(text, 1, 0, OffsetUnit::Utf16), 7);
        assert_eq!(idx.byte_offset(text, 1, 5, OffsetUnit::Utf16), 12);
    }

    #[test]
    fn test_emoji_utf16() {
        // 🦀 is 4 bytes in UTF-8, 2 code units in UTF-16, 1 code point
        let text = "a🦀b\n";
        let idx = LineIndex::new(text);

        // Byte layout: a(1) 🦀(4) b(1) \n(1) = 7 bytes
        // UTF-16 units: a(1) 🦀(2) b(1) = 4 units
        // Code points:  a(1) 🦀(1) b(1) = 3 code points

        // UTF-16 col 0 → byte 0 (a)
        assert_eq!(idx.byte_offset(text, 0, 0, OffsetUnit::Utf16), 0);
        // UTF-16 col 1 → byte 1 (start of 🦀)
        assert_eq!(idx.byte_offset(text, 0, 1, OffsetUnit::Utf16), 1);
        // UTF-16 col 3 → byte 5 (b, because 🦀 takes 2 UTF-16 units)
        assert_eq!(idx.byte_offset(text, 0, 3, OffsetUnit::Utf16), 5);

        // Code point col 1 → byte 1 (start of 🦀)
        assert_eq!(idx.byte_offset(text, 0, 1, OffsetUnit::CodePoint), 1);
        // Code point col 2 → byte 5 (b, after 🦀 which is 1 code point)
        assert_eq!(idx.byte_offset(text, 0, 2, OffsetUnit::CodePoint), 5);
    }

    #[test]
    fn test_code_point_to_utf16() {
        // 🦀 is 1 code point but 2 UTF-16 units
        let text = "a🦀b\n";
        let idx = LineIndex::new(text);

        // Code point col 0 → UTF-16 col 0
        assert_eq!(idx.code_point_to_utf16(text, 0, 0), 0);
        // Code point col 1 → UTF-16 col 1 (just 'a')
        assert_eq!(idx.code_point_to_utf16(text, 0, 1), 1);
        // Code point col 2 → UTF-16 col 3 ('a' + 🦀 = 1 + 2)
        assert_eq!(idx.code_point_to_utf16(text, 0, 2), 3);
        // Code point col 3 → UTF-16 col 4 ('a' + 🦀 + 'b' = 1 + 2 + 1)
        assert_eq!(idx.code_point_to_utf16(text, 0, 3), 4);
    }

    #[test]
    fn test_lsp_position_to_byte() {
        let text = "(define x 1)\n(+ x 2)\n";
        let idx = LineIndex::new(text);
        let pos = Position::new(1, 7); // end of "(+ x 2)"
        assert_eq!(idx.lsp_position_to_byte(text, pos), 20);
    }

    #[test]
    fn test_multiline_span() {
        // "你好" is 2 code points, each 1 UTF-16 unit
        // 🦀 is 1 code point, 2 UTF-16 units
        let text = "(define x\n  \"你好🦀\")\n";
        let idx = LineIndex::new(text);
        
        let start_line = 2; // 1-indexed for Racket
        let start_col = 2; // 0-indexed code points
        let span = 5; // code points: ", 你, 好, 🦀, "
        
        let range = idx.range_from_span(text, start_line, start_col, span);
        
        // LSP is 0-indexed.
        // Line 1 (0-indexed)
        assert_eq!(range.start.line, 1);
        assert_eq!(range.start.character, 2); // 2 spaces before "
        
        assert_eq!(range.end.line, 1);
        // " = 1, 你 = 1, 好 = 1, 🦀 = 2, " = 1 -> Total UTF-16 width = 6
        // character should be 2 (start) + 6 = 8
        assert_eq!(range.end.character, 8);
        
        let start_idx = idx.lsp_position_to_byte(text, range.start);
        let end_idx = idx.lsp_position_to_byte(text, range.end);
        assert_eq!(&text[start_idx..end_idx], "\"你好🦀\"");
    }

    #[test]
    fn test_beyond_last_line() {
        let text = "hello\n";
        let idx = LineIndex::new(text);
        // Line 99 doesn't exist — should return start of last known line
        let offset = idx.byte_offset(text, 99, 0, OffsetUnit::Utf16);
        assert_eq!(offset, 6); // start of line after "hello\n"
    }

    #[test]
    fn test_cjk_characters() {
        // CJK characters are 3 bytes in UTF-8, 1 UTF-16 code unit, 1 code point
        let text = "你好世界\n";
        let idx = LineIndex::new(text);

        // UTF-16 col 2 → byte 6 (start of '世')
        assert_eq!(idx.byte_offset(text, 0, 2, OffsetUnit::Utf16), 6);
        // Code point col 2 → byte 6 (same, since CJK is 1 code unit per code point)
        assert_eq!(idx.byte_offset(text, 0, 2, OffsetUnit::CodePoint), 6);
    }

    #[test]
    fn test_racket_char_indices() {
        let text = "a\r\nb🦀\rc\n";
        let mut chars = RacketCharIndices::new(text);
        assert_eq!(chars.next(), Some((0, "a")));
        assert_eq!(chars.next(), Some((1, "\r\n")));
        assert_eq!(chars.next(), Some((3, "b")));
        assert_eq!(chars.next(), Some((4, "🦀")));
        assert_eq!(chars.next(), Some((8, "\r")));
        assert_eq!(chars.next(), Some((9, "c")));
        assert_eq!(chars.next(), Some((10, "\n")));
        assert_eq!(chars.next(), None);
    }

    #[test]
    fn test_crlf_drift_stress() {
        // Racket normalizes CRLF to LF. 
        // Original: (a\r\n b) (bytes 0..7)
        // Normalized: (a\n b) (chars: index 0..5, span 6)
        // Let's say we have:
        // Line 1: "(a\r\n"
        // Line 2: " b)"
        let text = "(a\r\n b)";
        let idx = LineIndex::new(text);
        
        // Racket normalized text is "(a\n b)"
        // Span of "(a\n b)" is 6 characters.
        // Start: Line 1, Col 0.
        let span = 6; 
        let range = idx.range_from_span(text, 1, 0, span);
        
        // Final position should be AFTER ')'
        // In original "(a\r\n b)", ')' is at byte 6. Offset AFTER ')' is 7.
        let end_idx = idx.lsp_position_to_byte(text, range.end);
        
        // With the fix, CRLF is treated as 1 unit, so span 6 reaches offset 7.
        assert_eq!(end_idx, 7, "Drift detected: end_idx is {}, expected 7", end_idx);
    }
}
