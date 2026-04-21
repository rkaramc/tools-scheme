# Coordinate Mapping and `LineIndex`

This document describes the technical implementation and core capabilities of the coordinate mapping system in the Scheme Toolbox LSP. The central component of this system is the `LineIndex` struct.

## Overview

The LSP server must translate between three distinct coordinate systems to ensure that evaluation results from Racket are correctly displayed in VS Code.

| System | Context | Line Indexing | Column Units | CRLF Treatment |
| :--- | :--- | :--- | :--- | :--- |
| **Bytes** | Rust/String Slicing | N/A | 8-bit Bytes (UTF-8) | 2 Bytes |
| **LSP** | VS Code / Protocol | 0-indexed | UTF-16 Code Units | 2 Units |
| **Racket** | Syntax Objects | 1-indexed | Unicode Code Points | **1 Code Point** |

## Core Capabilities of `LineIndex`

The `LineIndex` struct provides the following essential capabilities:

### 1. Bidirectional Line Mapping
`LineIndex` maintains a pre-computed list of byte offsets representing the start of every line in a document.
*   **Forward**: `line_start(line: usize) -> usize` provides $O(1)$ lookup for the beginning of any line.
*   **Reverse**: `offset_to_position(offset: usize) -> Position` uses binary search ($O(\log N)$) to find the line number and column for any arbitrary byte offset.

### 2. Multi-Unit Column Resolution
The `byte_offset` function acts as a universal translator for columns within a line. It takes an `OffsetUnit` parameter to specify how to "count" the distance from the start of the line:
*   **`OffsetUnit::Utf16`**: Counts the column according to LSP rules (surrogate pairs count as 2).
*   **`OffsetUnit::CodePoint`**: Counts the column according to Racket rules (surrogate pairs count as 1).

### 3. Atomic CRLF Normalization
To match Racket's behavior—where `\r\n` is normalized to a single character during reading—the `LineIndex` uses a custom `RacketCharIndices` iterator. This iterator peeks ahead at `\r` characters and yields `\r\n` as a single atomic string slice if they appear together.
*   This ensures that a Racket column index remains in sync with the actual byte layout of the file, even on Windows.
*   Without this, Racket-reported coordinates would "drift" by one character for every line ending in a CRLF file.

### 4. Span-to-Range Translation
Racket reports locations using a **Span** (Start Position + Length in Code Points). The `range_from_span` capability handles the complex task of:
1.  Finding the byte offset for the Racket start position.
2.  Walking exactly $N$ "Racket characters" (Code Points) to find the ending byte offset.
3.  Converting both the start and end byte offsets into LSP `Position` objects (UTF-16).

## Implementation Guarantees

### Performance
*   **Initialization**: $O(N)$ where $N$ is the number of bytes in the document.
*   **Line Lookup**: $O(1)$.
*   **Column Resolution**: $O(L)$ where $L$ is the length of the line.
*   **Position Recovery**: $O(\log M + L)$ where $M$ is the number of lines.

### Safety and Robustness
*   **Clamping**: If a coordinate is requested beyond the end of a line or the end of the file, the system clamps the result to the nearest valid boundary rather than panicking.
*   **Boundary Awareness**: All translations respect Unicode character boundaries, preventing the creation of invalid byte offsets that split multi-byte characters.
