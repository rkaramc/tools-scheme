# Spec: Racket Notebook Interface for VS Code

## Objective
Implement a native VS Code Notebook experience for Racket/Scheme by leveraging the existing Scheme Toolbox LSP and `eval-shim.rkt`. This allows users to interleave rich markdown with executable code cells directly in `.rkt` files, with support for rich media (images), streaming output, and robust execution cancellation via `racket/sandbox`.

### User Stories
- As a student, I want to see the result of my Racket expressions (including images) immediately below the code cell.
- As a developer, I want to cancel long-running or infinite loops without restarting the entire LSP server.
- As a researcher, I want to use standard `.rkt` files that remain compatible with other editors while enjoying a notebook experience in VS Code.

## Tech Stack
- **Frontend:** VS Code Notebook API (TypeScript).
- **LSP Protocol:** Extended with custom notifications (`scheme/notebook/...`).
- **Backend (Rust):** Rust LSP server updated to handle asynchronous evaluation tasks.
- **Backend (Racket):** `eval-shim.rkt` refactored to use `racket/sandbox` for robust execution and `current-print` overrides for rich media serialization.

## Commands
### Build & Test
```powershell
# Build Rust LSP
cd lsp; cargo build

# Build VS Code Extension
cd editors/vscode; npm install; npm run compile

# Run tests
cd lsp; cargo test
cd editors/vscode; npm test
```

## Project Structure
```
editors/vscode/
├── src/
│   ├── notebook/
│   │   ├── serializer.ts   # Parses .rkt into NotebookData
│   │   ├── controller.ts   # Dispatches cells to LSP
│   │   └── renderer.ts     # (Optional) Custom output rendering
│   └── extension.ts        # Entry point registration
lsp/src/
├── server.rs               # LSP method dispatching
├── evaluator.rs            # Racket process management
├── worker.rs               # Async evaluation logic
└── eval-shim.rkt           # Racket execution engine
```

## LSP Interface Design

### 1. Custom Notifications
All notebook methods are grouped under the `scheme/notebook` namespace.

| Method | Type | Direction | Description |
| :--- | :--- | :--- | :--- |
| `scheme/notebook/evalCell` | Notification | Client -> Server | Starts evaluation of a cell. |
| `scheme/notebook/cancelEval` | Notification | Client -> Server | Cancels a running cell evaluation. |
| `scheme/notebook/outputStream` | Notification | Server -> Client | Streams output (text/image) to a cell. |
| `scheme/notebook/evalFinished` | Notification | Server -> Client | Signal that a cell has finished running. |

### 2. Data Structures
```typescript
interface EvalCellParams {
  uri: string;
  code: string;
  executionId: number; // Unique ID for routing results
}

interface NotebookOutputParams {
  executionId: number;
  payload: {
    type: 'stdout' | 'stderr' | 'rich' | 'error';
    mime?: string; // e.g. 'image/png'
    data: string;  // text or Base64 encoded media
  };
}
```

## Testing Strategy
- **Unit Tests (Rust):** Test JSON serialization and asynchronous task dispatching in `evaluator.rs`.
- **Unit Tests (Racket):** Test `eval-shim.rkt` output capture and image serialization.
- **Integration Tests:** Verify that running a cell in VS Code triggers the correct LSP notification and returns output.

## Boundaries
- **Always do:** Ensure `.rkt` files saved to disk are valid Racket modules (no proprietary metadata).
- **Ask first:** Before introducing new external Racket dependencies.
- **Never do:** Block the main LSP loop with evaluation tasks.

## Success Criteria
- Opening a `.rkt` file as a notebook correctly separates `#lang`, markdown comments, and code cells.
- Executing a code cell displays `stdout`, `stderr`, and the final value (if not `void`).
- `2htdp/image` objects are rendered as inline PNGs.
- Clicking the "Stop" button in VS Code successfully terminates the Racket execution thread.

## Open Questions
- What is the exact timeout limit we set on the sandbox for infinite loops? (Proposed: 15 seconds).
- How do we handle `read` (stdin) if a student's code prompts for user input? (MVP: Simply fail/timeout).
