# Spec: Notebook Source Parsing (ADR-002)

## Objective
Unify the parsing logic between Racket Notebooks and standard `.rkt` files by using empty lines (two or more consecutive newlines) as logical delimiters for code blocks. This provides a "Notebook-like" experience in the standard editor by generating a single CodeLens per physical block rather than per top-level form.

## Assumptions
- The user has chosen a **Racket-side split** strategy where the eval-shim handles the structural parsing (using a scan-first approach for markdown).
- The CodeLens generated for a valid block will span the **entire physical block**, matching ADR defaults.
- Markdown blocks enclosed in `#| markdown ... |#` are identified and treated as markdown cells.

## Tech Stack
- Rust (LSP Server in `lsp/`)
- Racket (Evaluation Shim in `lsp/src/eval-shim.rkt`)
- Regular Expressions (`regex` crate in Rust)

## Commands
- Build: `just debug`
- Test LSP: `just test lsp`

## Project Structure
- `lsp/src/worker.rs`: Handles the `on_parse` logic and CodeLens generation.
- `lsp/src/evaluator.rs`: API update to support block validation.
- `lsp/src/eval-shim.rkt`: Update to handle validation.
- `lsp/tests/integration.rs`: Integration tests for range splitting.

## Implementation Strategy

### Racket-Side Structural Parsing
1. In `eval-shim.rkt` `parse-string-content`, we use a **scan-first strategy**:
   - First, scan for structural markdown delimiters (`#| markdown` and `|#`) to isolate documentation zones.
   - Second, segment the remaining code text into physical blocks using the empty-line regex `(?m:(\r?\n[ \t]*){2,})`.
2. This ensures that markdown blocks containing empty lines are preserved as a single unit, matching the VS Code extension's behavior.

### Semantic Validation
1. **Unified Pipeline**: To minimize IPC overhead, we unified segmentation and validation.
2. The `parse` command in the REPL protocol now returns a stream of `range` objects, each containing:
   - Byte `pos` and `span`.
   - `kind` ("code" or "markdown").
   - `valid` boolean (true if the block contains at least one evaluable Racket form).
3. Rust uses the `LineIndex` to convert these byte offsets into precise LSP ranges.

## Testing Strategy
- Add unit tests in Rust to verify the block regex splitting correctly computes offsets.
- Update `lsp/tests/integration.rs` to verify:
  1. Two `(define ...)` forms separated by a single newline produce *one* CodeLens.
  2. Two `(define ...)` forms separated by two newlines produce *two* CodeLenses.
  3. A block containing only comments produces *zero* CodeLenses.

## Boundaries
- **Always**: Compute accurate byte/char offsets using `LineIndex`.
- **Never**: Break existing evaluation logic in `eval-shim.rkt`; only add the new validation command.

## Tasks
- [x] Task 1: Update `evaluator.rs` and `eval-shim.rkt` to support a `validate-blocks` IPC command that accepts an array of strings and returns a boolean array.
- [x] Task 2: Implement regex-based block splitting in `worker.rs` `on_parse`, tracking precise start and end `Position`s for each block.
- [x] Task 3: Integrate markdown detection to generate a markdown range but skip executable CodeLens.
- [x] Task 4: Integrate `validate-blocks` in `on_parse` to filter the blocks, and assign the full block ranges to `doc.ranges`.
- [x] Task 5: Write tests in `integration.rs` to verify the new grouping behavior.
