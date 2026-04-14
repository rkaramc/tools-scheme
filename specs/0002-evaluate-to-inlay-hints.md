# Spec: Evaluate Scheme/Racket to Inlay Hints

## Objective
Enable users to evaluate Scheme/Racket source code directly from the editor and view the results of each top-level expression as inlay hints. This provides immediate feedback without needing a separate REPL window.

## Tech Stack
- **LSP Server**: Rust (`scheme-toolbox-lsp`)
- **Execution Engine**: Racket (external subprocess)
- **Editor**: VS Code (Primary for testing), Helix, Zed
- **LSP Features**: `textDocument/codeAction`, `textDocument/inlayHint`, `textDocument/publishDiagnostics`

## Commands
- **Evaluate File**: Triggered via Code Action or Command Palette.
- **Build LSP**: `cargo build -p scheme-toolbox-lsp`
- **Run Tests**: `cargo test -p scheme-toolbox-lsp`

## Project Structure
```
lsp/src/
├── main.rs          → Entry point & LSP loop
├── evaluator.rs     → Logic for calling `racket` and parsing output
├── parser.rs        → S-expression boundary detection
└── inlay_hints.rs   → Mapping evaluation results to LSP InlayHint types
```

## Implementation Strategy
1. **Trigger**: User selects "Evaluate File" Code Action.
2. **Execution**:
   - LSP saves a temporary version of the current buffer.
   - LSP executes `racket -e '(load "temp_file.rkt")'` or uses a long-running REPL process to maintain state.
   - LSP captures `stdout` (results) and `stderr` (errors).
3. **Parsing**:
   - LSP identifies top-level S-expressions in the source.
   - LSP maps the captured results from Racket back to the line numbers of these expressions.
4. **Display**:
   - Successful evaluation results are sent via `textDocument/inlayHint` at the end of the corresponding expression.
   - Evaluation errors are published as `Diagnostic` objects (red-underline). Error diagnostics are considered superior to inlay hints for feedback on failing expressions, as they integrate with the editor's error list and provide standard visual cues.
   - Inlay hints are suppressed for expressions that produce errors to avoid visual clutter and redundant feedback.

## Code Style (Rust)
```rust
/// Represents the result of evaluating a single top-level expression.
pub struct EvalResult {
    pub line: u32,
    pub content: String,
    pub is_error: bool,
}

// Use explicit matching for LSP types
match result {
    Ok(val) => InlayHint { 
        label: InlayHintLabel::String(format!("=> {}", val)),
        kind: Some(InlayHintKind::PARAMETER), // Visual styling hint
        ..Default::default()
    },
    Err(e) => InlayHint { ... }
}
```

## Testing Strategy
- **Unit Tests**: Test the S-expression parser against various Scheme/Racket syntax variants.
- **Integration Tests**: Mock the `racket` subprocess to verify that the LSP correctly maps process output to line-specific inlay hints.
- **Manual Verification**: Use VS Code Extension Development Host to verify visual placement and persistence of state (e.g., defining a variable in line 1 and using it in line 5).

## Boundaries
- **Always**: Use a sandbox or temporary file for evaluation to avoid corrupting user source.
- **Ask first**: Before adding heavy dependencies for parsing (e.g., `tree-sitter`).
- **Never**: Block the main LSP thread while waiting for the Racket process to complete (always use async or separate threads).

## Success Criteria
- [x] LSP identifies top-level expressions in a `.rkt` or `.scm` file.
- [x] User can trigger evaluation via an editor action.
- [x] Successful evaluation results appear as inlay hints at the end of the lines.
- [x] State is persistent (definitions are available to subsequent expressions).
- [x] Evaluation errors generate red-underline diagnostics.
- [x] Inlay hints are hidden for expressions that contain evaluation errors.
