# Spec: Split EvalWorker into EvaluationActor and AnalysisActor (ts-043.1)

## Objective
Address the "God Actor Congestion" (Critical Risk #1) identified in the architectural analysis. Currently, the `EvalWorker` handles both fast, synchronous parsing (which updates CodeLenses) and slow, blocking evaluations (which update Diagnostics and Output). If a user evaluates an infinite loop or a 10-second script, the IDE stops updating CodeLenses for new text because parsing is stuck behind evaluation in the single MPSC mailbox. 

By splitting this into two distinct actors (`EvaluationActor` and `AnalysisActor`), we ensure parsing and UI feedback remain instantly responsive, even during heavy evaluation.

## Architecture & Assumptions
**Racket-Side Interaction:** There is **NO** shared state on the Racket side between parsing and evaluation. 
- **Evaluation** creates isolated sandboxes and modifies namespaces. 
- **Analysis** (`validate-blocks`) merely calls `read-syntax` on ephemeral strings to check if they are syntactically valid forms, which does not mutate the Racket environment. 
- **Assumption:** We will spawn **Two Dedicated Racket Processes** (one for each actor) to completely eliminate contention.

**Rust-Side Interaction:** Yes, there is shared state on the Rust side.
1. **Shared Document Store:** Both actors will hold an `Arc<RwLock<SharedState>>`. The `AnalysisActor` writes to `doc.ranges` (CodeLenses), while the `EvaluationActor` writes to `doc.results` (Diagnostics/InlayHints). They will briefly compete for the `write` lock when updating their respective fields, but because the lock scope is extremely narrow (just assigning a `Vec`), contention will be negligible.
2. **LSP Message Sender:** Both actors will hold a clone of the `crossbeam_channel::Sender<Message>` to send notifications (Diagnostics) and requests (Refresh CodeLens/InlayHints) back to the client.

## Tech Stack
- Rust (LSP Server in `lsp/`)
- `crossbeam_channel` for actor mailboxes

## Commands
- Build: `just debug`
- Test LSP: `just test lsp`

## Project Structure
- `lsp/src/worker.rs`: The single `eval_worker` function will be split into `eval_worker` and `analysis_worker` (or we'll use a generic worker function with different channels).
- `lsp/src/server.rs`: Will spawn two threads (one for eval, one for analysis) and maintain two `Sender` channels (`eval_tx` and `analysis_tx`).

## Implementation Strategy

### 1. Split Channels and State
- In `server.rs`, `ServerState` currently has `eval_tx: crossbeam_channel::Sender<EvalTask>`. 
- We will add `analysis_tx: crossbeam_channel::Sender<EvalTask>`.
- In `main.rs` (or wherever `eval_worker` is spawned), we will instantiate *two* `Evaluator`s: one for `eval` and one for `analysis`.
- We will spawn two OS threads. One runs `eval_worker(eval_tx_receiver)`, the other runs `analysis_worker(analysis_tx_receiver)`.

### 2. Message Routing (`server.rs`)
- `didChange`, `scheme.parse` → Route `EvalAction::Parse` to `analysis_tx`.
- `scheme.evaluate`, `scheme.evaluateSelection`, `scheme.evalCell` → Route `EvalAction::Evaluate`/`EvalCell` to `eval_tx`.
- `scheme.clearNamespace` → Route `EvalAction::Clear` to **only** `eval_tx` (since analysis has no persistent namespace).
- `scheme.restartREPL` → Route `EvalAction::Restart` to **both** `eval_tx` and `analysis_tx` to completely refresh both underlying Racket processes.

### 3. Worker Logic (`worker.rs`)
- Separate the massive `eval_worker` match loop into two focused loops: `eval_worker` handles evaluate/cell/clear/restart, and `analysis_worker` handles parse/restart.
- Both workers will need access to `state` (`Arc<RwLock<SharedState>>`) and `sender` (LSP client messages).

## Testing Strategy
- Ensure all 18 existing `integration.rs` tests pass without modification (they treat the server as a black box).
- Verify that `test_notebook_eval`, `test_notebook_uri_isolation`, and `test_notebook_state_persistence` remain stable under the new dual-process model.

## Boundaries
- **Always**: Keep `RwLock` acquisitions brief.
- **Never**: Share the same `Evaluator` instance between the two threads, as that would re-introduce the exact blocking behavior we are trying to fix.

## Tasks
- [x] Task 1: Update `worker.rs` to separate `eval_worker` into `eval_worker` and `analysis_worker` functions, each handling their respective `EvalAction` variants.
- [x] Task 2: Update `server.rs` (or `main.rs` depending on where spawning occurs) to create two `Evaluator` instances, two MPSC channels (`eval_tx`, `analysis_tx`), and spawn the two worker threads.
- [x] Task 3: Update `server.rs` message handling to route `Parse` to `analysis_tx`, `Evaluate`/`Clear` to `eval_tx`, and `Restart` to both.
- [x] Task 4: Run `cargo test --test integration` to ensure no regressions in evaluation or parsing logic.