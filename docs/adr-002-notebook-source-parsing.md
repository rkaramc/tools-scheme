# ADR-002: Notebook and Source Code Parsing Strategy

This document outlines the architecture and implementation strategy for parsing Racket source files and notebook cells, specifically focusing on the use of empty lines as logical delimiters for code blocks.

## 1. Overview

The goal is to provide a consistent experience between standard Racket files (`.rkt`) and Racket Notebooks. A key UX requirement is that users should be able to group related forms into a single "cell" or "block" for evaluation and visualization. We have decided to use **empty lines** (two or more consecutive newlines) as the primary mechanism for delineating these blocks.

## 2. Current Implementation

The current implementation is fragmented between the VS Code extension and the Language Server Protocol (LSP) server.

### VS Code Extension (`editors/vscode/`)
- **`NotebookSerializer`**: Correctlies implements the cell-breaking logic during deserialization of `.rkt` files into notebooks.
- **Logic**: It uses `/(?:\r?\n[ \t]*){2,}/` to split code blocks and `/#\|\s*markdown\s*\n?/g` to identify and extract markdown cells.
- **Result**: When a `.rkt` file is opened as a notebook, each block of code separated by an empty line becomes a separate interactive cell.

### LSP Server (`lsp/`)
- **Parsing**: The LSP server currently uses Racket's `read-syntax` in `eval-shim.rkt` to identify top-level forms.
- **Problem**: `read-syntax` ignores all whitespace, including empty lines. This means every individual S-expression is treated as a standalone entity for CodeLenses and diagnostics.
- **CodeLens**: In a standard editor, every single `(define ...)` or `(check-equal? ...)` gets its own "Run" lens, leading to visual clutter when many small forms are grouped together.

## 3. Ideal Parse Procedure

To unify the experience, the parsing logic should follow this "Ideal Procedure" for both notebooks and source files:

### Step 1: Physical Block Segmentation
The raw text buffer should first be segmented into **physical blocks** based on the empty-line delimiter:
- **Delimiter**: `\n\n+` (optionally allowing horizontal whitespace between newlines).
- **Notebooks**: Each physical block maps directly to one VS Code Notebook Cell.
- **Source Files**: Each physical block maps to one "Logical Code Block".

### Step 2: Markdown and Documentation
In addition to code blocks, the system supports embedded documentation via multi-line comments.
- **Delimiter**: `#| markdown` ... `|#`.
- **Notebooks**: These blocks are stripped of the `#| markdown` and `|#` tokens and rendered as **Markup Cells**.
- **Source Files**: These blocks remain as standard Racket multi-line comments but can be used by the LSP to provide rich hover information or documentation previews.

### Step 3: Semantic Validation
Each physical block (that is not a markdown block) is then passed to the Racket reader (`read-syntax`):
- If the block contains zero valid forms (e.g., only comments or whitespace), it is discarded or treated as a documentation block.
- If the block contains one or more valid forms, it is treated as a single unit for evaluation.

### Step 4: Range Assignment
- **Notebooks**: The range is the entire cell.
- **Source Files**: The range is the span of the physical block. A **single CodeLens** is placed at the start of this block, which evaluates all forms within that block when clicked.

## 4. Remaining Work (ts-79s / ts-9al)

The following tasks are required to align the LSP with this ADR:

1.  **Update `eval-shim.rkt`**: Modify `parse-string-content` to first split by the empty-line regex before calling `for-each-syntax`. It should also detect `#| markdown |#` blocks and return them with a specific `type: "markdown"` range.
2.  **LSP Worker Logic**: Update `on_parse` in `worker.rs` to handle these grouped ranges and ignore markdown blocks for evaluation tasks.
3.  **CodeLens Refactoring**: Modify `server.rs` (`handle_code_lens`) to ensure it only generates one lens per logical block.
4.  **Integration Tests**: Add tests to `integration.rs` verifying that multiple forms separated by a single newline get one lens, while forms separated by two newlines get two separate lenses.

## 5. Decision and Rationale

We choose **empty lines** over explicit delimiters (like `# %%` used in Python) to maintain idiomatic Racket style. Racket programmers already use empty lines to group related functions or tests; by making this semantic, we provide a natural "Notebook-like" experience in the standard editor without forcing new syntax on the user.

## 6. References
- [ts-79s]: parse notebook source code files to treat empty lines as notebook cell breaks
- [ts-9al]: revise codelens generation for standard text editor to group by empty lines
- [ADR-001]: Racket to VS Code Coordinate Mapping
