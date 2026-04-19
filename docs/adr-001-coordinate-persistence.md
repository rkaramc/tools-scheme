# Racket to VS Code Coordinate Mapping Investigation & Implementation

This document details the investigation and final implementation logic for mapping Racket's internal syntax coordinates to LSP-compliant UTF-16 positions used by VS Code for inlay hints and diagnostics.

## 1. Overview

Racket and the Language Server Protocol (LSP) use different systems for measuring positions in a text file:

| Metric | Racket Syntax Objects | LSP (VS Code) |
| :--- | :--- | :--- |
| **Line Indexing** | 1-indexed | 0-indexed |
| **Column Indexing** | 0-indexed | 0-indexed |
| **Column Units** | Unicode Code Points | UTF-16 Code Units |
| **Line Endings** | CRLF counted as **1** character | CRLF counted as **2** characters |
| **Position** | 1-indexed character count | N/A (uses Line/Character) |

## 2. Investigation: Racket's Coordinate System

Empirical testing with `read-syntax` on Windows-style (CRLF) and multibyte (Emoji) files revealed how Racket calculates its metrics.

### Key Finding: The CRLF Rule
Racket's reader treats the sequence `\r\n` as a **single character** for `syntax-line`, `syntax-column`, and `syntax-position`.

**Example:**
Input string: `(\r\n  x)`
- `(` is at line 1, col 0, pos 1.
- `\r\n` increments the line to 2 and resets col to 0, but only increments `pos` by 1.
- ` ` (space 1) is at line 2, col 0, pos 3.
- ` ` (space 2) is at line 2, col 1, pos 4.
- `x` is at line 2, col 2, pos 5.

### Multibyte Characters
Racket counts Unicode code points. An emoji like 🦀 is 1 character/codepoint in Racket, but 2 code units in UTF-16 (LSP) and 4 bytes in UTF-8.

## 3. Implementation Details

The mapping logic is implemented across three main layers in the `lsp` component.

### Layer 1: `coordinates.rs` (The Foundation)
The `LineIndex` struct provides the core translation logic. It pre-computes line starts (byte offsets) and offers a `byte_offset` method that can walk a line using different `OffsetUnit` types.

```rust
// Respecting Racket's CRLF rule in byte_offset calculation
if unit == OffsetUnit::CodePoint {
    if c == '\r' && chars.peek().map(|&(_, next_c)| next_c) == Some('\n') {
        let _ = chars.next(); // Consume the '\n' but count it as 1 unit
    }
    1
}
```

### Layer 2: `server.rs` (The Normalization)
When the Racket evaluator returns results, they are immediately passed through `normalize_results`. This process has two phases:

1.  **Anchor to Bytes**: We use Racket's `line` and `col` (code points) to find the absolute **byte offset** in the document text. We store this as a 1-indexed `res.pos`.
2.  **Derive LSP Coordinates**: Using the stable byte offset and the Unicode `span` (number of code points), we walk the text to find the start and end **UTF-16 columns** required by the LSP.

```rust
fn recalculate_from_byte_pos(results: &mut [EvalResult], text: &str, line_index: &LineIndex) {
    for res in results.iter_mut() {
        let pos_byte_idx = res.pos.saturating_sub(1) as usize;
        let mut end_byte_idx = pos_byte_idx;
        let mut chars = text[pos_byte_idx..].chars().peekable();
        
        for _ in 0..res.span {
            if let Some(c) = chars.next() {
                end_byte_idx += c.len_utf8();
                if c == '\r' && chars.peek() == Some(&'\n') {
                    end_byte_idx += chars.next().unwrap().len_utf8();
                }
            } else { break; }
        }
        
        let start_pos = line_index.offset_to_position(text, pos_byte_idx);
        let end_pos = line_index.offset_to_position(text, end_byte_idx);
        
        res.line = start_pos.line + 1; // Standardized internal representation
        res.col = start_pos.character;  // Now in UTF-16 units
    }
}
```

### Layer 3: `inlay_hints.rs` (The Display)
By the time results reach the inlay hint generator, they are already normalized to 0-indexed line/character (UTF-16) coordinates. The generator simply maps these to `lsp_types::InlayHint`.

## 4. Handling Selection Offsets
When a user evaluates a selection, Racket reports coordinates relative to the start of that selection (e.g., Line 1, Col 0). The `eval_worker` in `server.rs` shifts these byte offsets by the selection's start byte before running the normalization. This ensures that even for partial files, the coordinates map back to the correct global position in the editor.

## 5. Sources and References

*   **Racket Documentation**: [Syntax Object Properties](https://docs.racket-lang.org/reference/stxprops.html) - Details on `syntax-line`, `syntax-column`, and `syntax-position`.
*   **LSP Specification**: [Position](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#position) - Defines the UTF-16 code unit requirement for columns.
*   **Rust `std::str`**: [Character Indices](https://doc.rust-lang.org/std/primitive.str.html#method.char_indices) - Used for safe Unicode iteration.
*   **Internal Tests**:
    *   `lsp/tests/encoding_test.rs`: Verifies emoji and multiline positioning.
    *   `lsp/src/coordinates.rs`: Contains unit tests for `test_crlf_drift_stress`.
