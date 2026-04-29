# ADR-002: Notebook and Source Code Parsing Strategy

This document outlines the architecture and implementation strategy for parsing Racket source files and notebook cells, specifically focusing on the use of empty lines as logical delimiters for code blocks.

## 1. Overview

The goal is to provide a consistent experience between standard Racket files (`.rkt`) and Racket Notebooks. A key UX requirement is that users should be able to group related forms into a single "cell" or "block" for evaluation and visualization. We have decided to use **empty lines** (two or more consecutive newlines) as the primary mechanism for delineating these blocks.

## 2. Legacy Implementation (Pre-Refactor)

Prior to this architecture, the implementation was fragmented and relied on different parsing models:

### LSP Server (`lsp/`)
- **S-expression Parsing**: The server used Racket's `read-syntax` directly. Since the reader ignores whitespace, every individual S-expression (e.g., each `define`) was treated as a standalone entity.
- **CodeLens Spam**: This resulted in excessive "Run" lenses in the standard editor, as every single top-level form received its own button, even when grouped together.
- **No Markdown Awareness**: Documentation blocks (`#| markdown |#`) were treated as regular comments and skipped by the LSP metadata pass.

### VS Code Extension (`editors/vscode/`)
- **Fragmented Logic**: The `NotebookSerializer` correctly grouped code by empty lines, but this logic was entirely decoupled from the LSP.
- **Inconsistency**: A file opened as a Notebook and the same file opened as Source Code had different logical boundaries, leading to a disjointed user experience.

## 3. Current Implementation

### VS Code Extension (`editors/vscode/`)
- **`NotebookSerializer`**: Implements the cell-breaking logic during deserialization of `.rkt` files into notebooks.
- **Strategy**: Uses a **scan-first** approach: it identifies `#| markdown |#` blocks first, then segments the remaining code using the empty-line regex `(?m:(\r?\n[ \t]*){2,})`.
- **Role**: Essential for building the Notebook UI cells independently of the LSP.

### Unified Racket-Side Parsing (`lsp/`)
- **`eval-shim.rkt`**: Implements an identical **scan-first** structural parser to the VS Code extension.
- **Protocol**: The `parse` command returns unified metadata (`kind`, `valid`, `pos`, `span`).
- **Rust Integration**: `worker.rs` delegates all parsing to Racket and uses `LineIndex` for precise coordinate mapping.
- **Consistency**: This approach ensures that `.rkt` files opened in standard editors and the VS Code Notebook UI see identical cell/block boundaries.

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

The following tasks have been completed:

1.  **[x] Update `eval-shim.rkt`**: Refactored `parse-string-content` with a scan-first structural strategy and flexible markdown regex.
2.  **[x] LSP Worker Logic**: Refactored `on_parse` in `worker.rs` to use unified Racket parsing and accurate byte-to-range mapping.
3.  **[x] CodeLens Refactoring**: Standardized CodeLens generation to use the unified logical blocks.
4.  **[x] Integration Tests**: Added comprehensive cross-language unit tests ensuring equivalent parsing across VS Code and Racket.

## 5. Decision and Rationale

We choose **empty lines** over explicit delimiters (like `# %%` used in Python) to maintain idiomatic Racket style. Racket programmers already use empty lines to group related functions or tests; by making this semantic, we provide a natural "Notebook-like" experience in the standard editor without forcing new syntax on the user.

## 6. References
- [ts-79s]: parse notebook source code files to treat empty lines as notebook cell breaks
- [ts-9al]: revise codelens generation for standard text editor to group by empty lines
- [ADR-001]: Racket to VS Code Coordinate Mapping
