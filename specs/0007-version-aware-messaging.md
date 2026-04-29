# Spec: Implement version-aware messaging to fix coordinate drift (ts-043.2)

## Objective
Address the "Coordinate Drift" (High Risk #2) identified in the architectural analysis. Coordinate drift happens because `EvalWorker` translates char-based locations returned by Racket using the *current* `LineIndex` from `DocumentStore`. If a user types while Racket is busy, the document version increases. If `EvalWorker` then applies older evaluation results, the CodeLenses, InlayHints, and Diagnostics will be displayed on the wrong lines. 

By implementing Pre-Flight and Post-Flight version checks in Rust, we will cleanly drop outdated parsing and evaluation results, preventing stale UI updates.

## Architecture & Assumptions
- **Rust-Side Checks:** The user has confirmed we will use Rust-side checks. `worker.rs` already receives the target `version` in its `EvalTask`.
- **Pre-Flight Check:** Before invoking the blocking Racket evaluation/parsing, check if `doc.version > task.version`. If so, skip the expensive operation entirely.
- **Post-Flight Check:** After Racket returns, acquire the `write` lock to the DocumentStore. Check again if `doc.version > task.version`. If so, discard the results and do not publish diagnostics or trigger a CodeLens/InlayHint refresh.

## Tech Stack
- Rust (LSP Server in `lsp/`)

## Commands
- Build: `just debug`
- Test LSP: `just test lsp`

## Project Structure
- `lsp/src/worker.rs`: The sole file requiring modification. `on_parse` and `on_evaluate` will be updated with the version guard logic.

## Implementation Strategy

### 1. `on_parse` Updates
- **Pre-flight:** Update the initial `read_recovered` block. Currently, it logs "Skipping parse: newer version already in store" and returns. This logic is already correct! We just need to make sure the post-flight logic exists.
- **Post-flight:** After Racket finishes validating blocks, inside the `write_recovered` block, re-verify `doc.version > version`. If true, `return` before mutating `doc.ranges` and before calling `sender.refresh_code_lenses()`.

### 2. `on_evaluate` Updates
- **Pre-flight:** Add a `read_recovered` check before calling `evaluator.evaluate_str`. If the document's version is greater than the requested `version`, log a cancellation message and `return` early to save Racket processing time.
- **Post-flight:** Inside the `write_recovered` block, check `doc.version > version.unwrap_or(0)` (or simply check if `doc.version != v`). If outdated, `return` before applying `merge_results`, and skip the subsequent `send_diagnostics` and `refresh_inlay_hints` calls.

### 3. `on_eval_cell` Updates
- For notebook cells, the output stream notifications (`scheme/notebook/outputStream`) are stateless in the LSP and handled by VS Code's robust cell versioning. Diagnostics are generated at the end. We should add a post-flight check to drop the final `send_diagnostics` call if the document version has drifted.

## Testing Strategy
- Run `cargo test --test integration` to ensure no existing logic is broken.
- Introduce a specific integration test `test_coordinate_drift_prevention` that:
  1. Opens a document (version 1) and triggers an evaluation block (using a slow or infinite loop).
  2. Immediately sends a `didChange` to bump the version to 2.
  3. Verifies that the evaluation response for version 1 does *not* result in diagnostics or inlay hints being published for version 1.

## Boundaries
- **Always**: Use `unwrap_or(0)` or safe Option handling since `version` is an `Option<i32>`.
- **Never**: Alter the Racket JSON protocol, as Rust-side checks are sufficient and more performant.

## Tasks
- [x] Task 1: Add Pre-Flight and Post-Flight version checks to `on_evaluate` in `worker.rs`.
- [x] Task 2: Add Post-Flight version check to `on_parse` in `worker.rs` (Pre-Flight already exists).
- [x] Task 3: Add Post-Flight version check to `on_eval_cell` in `worker.rs`.
- [x] Task 4: Create integration test for coordinate drift prevention.
