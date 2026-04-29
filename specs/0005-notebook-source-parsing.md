# Spec: Notebook Source Parsing (ADR-002)

## Objective
Unify the parsing logic between Racket Notebooks and standard `.rkt` files by using empty lines (two or more consecutive newlines) as logical delimiters for code blocks. This provides a "Notebook-like" experience in the standard editor by generating a single CodeLens per physical block rather than per top-level form.

## Assumptions
- The user has chosen a **Rust-side split** strategy where Rust handles the regex-based chunking of the file buffer.
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

### Rust-Side Splitting
1. In `worker.rs` `on_parse`, instead of sending the entire document content to `evaluator.parse_str`, use a regex `(?m)(?:\r?\n[ \t]*){2,}` to split the `content` string into physical blocks.
2. Keep track of the byte offset and line/col start/end for each block using `LineIndex`.

### Markdown Detection
1. For each block, check if it contains the markdown delimiter `#| markdown` and `|#`.
2. If it is a markdown block, generate a range with a `markdown` flag but skip generating an executable CodeLens for it.

### Semantic Validation
1. **Latency Consideration**: To minimize IPC overhead while keeping responsiveness high, we will implement a batch-request, streaming-response validation approach.
2. Add a new `validate-blocks` command to `eval-shim.rkt`.
3. Rust will send a single JSON payload containing an array of block strings: `{"type": "validate-blocks", "blocks": [...]}`.
4. Racket will use `read-syntax` on each block sequentially. Instead of waiting for all blocks to be processed, Racket will stream the validation result back immediately for each block (e.g., `{"index": 0, "valid": true}`) as it comes in.
5. Rust listens for the streamed responses, collecting the valid blocks. For every `true` block, it generates an executable CodeLens range spanning the *entire* physical block. When `READY` is received, it finalizes the ranges.

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
