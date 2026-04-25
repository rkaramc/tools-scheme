# Tasks: Racket Notebook Implementation

## Phase 1: Racket Engine Refactor
- [ ] **Task 1: Implement Sandbox Evaluator**
  - Description: Refactor `eval-shim.rkt` to use `racket/sandbox` for per-document isolated evaluation.
  - Acceptance: `make-evaluator` is used to create namespaces; timeouts and resource limits are configurable.
  - Verify: Run `racket eval-shim.rkt --repl` and verify basic evaluation works.
  - Files: `lsp/src/eval-shim.rkt`

- [ ] **Task 2: Rich Media Serialization**
  - Description: Implement `current-print` override to serialize `2htdp/image` snips to Base64 PNGs.
  - Acceptance: Evaluating `(circle 10 "solid" "red")` returns a JSON object with `mime: "image/png"`.
  - Verify: Manual test with `racket/gui` and `2htdp/image` loaded.
  - Files: `lsp/src/eval-shim.rkt`

- [ ] **Task 3: Line-based JSON Protocol**
  - Description: Ensure all shim output (stdout/stderr/results) is wrapped in JSON and emitted line-by-line.
  - Acceptance: No raw text is emitted to stdout; everything is a structured JSON payload.
  - Verify: `evaluator.rs` tests updated to handle the new protocol.
  - Files: `lsp/src/eval-shim.rkt`, `lsp/src/evaluator.rs`

## Phase 2: Rust LSP Backend
- [ ] **Task 4: Async Notebook Notifications**
  - Description: Add `scheme/notebook/evalCell` and `scheme/notebook/cancelEval` handlers.
  - Acceptance: LSP server dispatches notebook tasks to the worker without blocking.
  - Verify: Log verification in `global.session`.
  - Files: `lsp/src/server.rs`, `lsp/src/worker.rs`

- [ ] **Task 5: Output Stream Forwarding**
  - Description: Forward JSON payloads from the Racket shim to VS Code as `scheme/notebook/outputStream`.
  - Acceptance: VS Code receive notifications for stdout and rich media during execution.
  - Verify: Integration test in `lsp/tests/integration.rs`.
  - Files: `lsp/src/worker.rs`, `lsp/src/evaluator.rs`

## Phase 3: VS Code Frontend
- [ ] **Task 6: Notebook Serializer**
  - Description: Implement `NotebookSerializer` to map `.rkt` files to VS Code cells.
  - Acceptance: Opening a `.rkt` file displays code and markdown cells correctly.
  - Verify: Manual check of file opening/saving.
  - Files: `editors/vscode/src/notebook/serializer.ts`

- [ ] **Task 7: Notebook Controller**
  - Description: Implement `NotebookController` to manage execution lifecycle.
  - Acceptance: Clicking "Run" sends code to LSP; output appears in the notebook.
  - Verify: Manual verification with standard and rich output.
  - Files: `editors/vscode/src/notebook/controller.ts`, `editors/vscode/src/extension.ts`

- [ ] **Task 8: Diagnostic Scoping**
  - Description: Scoped diagnostic degradation for notebook-opened files.
  - Acceptance: "Duplicate identifier" errors become Warnings in notebooks but stay Errors in standard files.
  - Verify: Check "Problems" tab in both views.
  - Files: `lsp/src/server.rs`
