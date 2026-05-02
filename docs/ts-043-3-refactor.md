# Implementation Plan: Refactoring SharedState RwLock (ts-043.3)

## Objective
Address lock contention and race conditions by transitioning the `SharedState` (`DocumentStore`) to be strictly owned by the LSP Gateway (the `Server` struct loop), effectively eliminating the `Arc<RwLock>` wrapper. Background workers will operate purely on immutable `DocumentSnapshot`s and send asynchronous results back to the Gateway via a new channel. This completes the transition to a purely Actor-based model (Gateway-owned DocStore).

## Scope
* Remove `RwLockExt` and `SharedState` wrapping from `server.rs`.
* Replace `Server.state: Arc<RwLock<SharedState>>` with `document_store: DocumentStore`.
* Introduce a `WorkerResult` enum to carry execution results from the `EvalWorker` and `AnalysisWorker` back to the Gateway.
* Update `Server::main_loop` to use `crossbeam_channel::select!` to multiplex between incoming LSP `Message`s and `WorkerResult`s.
* Refactor `worker.rs` to stop passing `Arc<RwLock<SharedState>>` and instead pass a `result_tx` sender.
* Move the `merge_results` logic and the building/sending of `PublishDiagnostics`, `inlayHint/refresh`, and `codeLens/refresh` out of the workers and into the Gateway, ensuring that all document mutations and UI update triggers occur sequentially on the main thread.

## Implementation Steps

1. **Define `WorkerResult` (in `server.rs` or `worker.rs`)**:
   ```rust
   pub enum WorkerResult {
       EvaluateComplete {
           uri: String,
           version: Option<i32>,
           results: Vec<EvalResult>,
           diagnostics: Vec<Diagnostic>,
           byte_range: Option<(u32, u32)>,
       },
       ParseComplete {
           uri: String,
           version: i32,
           ranges: Vec<Range>,
       },
       ClearNamespace {
           uri: String,
       },
       RestartComplete,
       CellEvaluationComplete {
           uri: String,
           version: Option<i32>,
           diagnostics: Vec<Diagnostic>,
       },
       EvaluationError {
           uri: String,
           version: Option<i32>,
           diagnostics: Vec<Diagnostic>,
       }
   }
   ```

2. **Update `Server` Struct (`server.rs`)**:
   * Change `state: Arc<RwLock<SharedState>>` to `pub document_store: DocumentStore`.
   * Add `pub sender: DiagnosticWorkerSender` to the `Server` struct.
   * Add a `handle_worker_result` method. This method will receive the `WorkerResult`. For `EvaluateComplete`, it will call `merge_results` on the main thread to update `DocumentStore.results`, and then dispatch the pre-computed `diagnostics`.

3. **Rewrite `main_loop` (`server.rs`)**:
   ```rust
   pub fn main_loop(
       &mut self, 
       connection: &lsp_server::Connection, 
       worker_rx: &crossbeam_channel::Receiver<WorkerResult>
   ) -> Result<LoopAction, Box<dyn Error + Sync + Send>> {
       let mut shutting_down = false;
       loop {
           crossbeam_channel::select! {
               recv(&connection.receiver) -> msg => {
                   match msg {
                       Ok(msg) => /* handle request/notification */,
                       Err(_) => break, // Connection closed
                   }
               }
               recv(worker_rx) -> msg => {
                   if let Ok(worker_res) = msg {
                       self.handle_worker_result(worker_res);
                   }
               }
           }
       }
       Ok(LoopAction::Continue)
   }
   ```

4. **Refactor `worker.rs`**:
   * Remove `state: Arc<RwLock<SharedState>>` from `eval_worker`, `analysis_worker`, and their internal handlers.
   * Pass `result_tx: crossbeam_channel::Sender<WorkerResult>` into the workers.
   * **CRITICAL LATENCY MITIGATION:** The background workers MUST retain the heavy processing.
     * In `on_evaluate`: Use the provided `DocumentSnapshot` to perform `normalize_results` or `recalculate_from_byte_pos`. After coordinate normalization, generate the `Vec<Diagnostic>`. Send BOTH the normalized `Vec<EvalResult>` and `Vec<Diagnostic>` via `WorkerResult::EvaluateComplete` to `result_tx`.
     * In `on_parse`: Calculate `Range`s using the snapshot and send `WorkerResult::ParseComplete`.
     * In `on_eval_cell`: Collect diagnostics and send `WorkerResult::CellEvaluationComplete`.
   * Remove `merge_results` from `worker.rs` entirely; this logic moves to `server.rs`.

5. **Refactor `main.rs`**:
   * Setup `let (result_tx, result_rx) = crossbeam_channel::unbounded();`.
   * Initialize `Server` with `DocumentStore::new()` directly, passing `result_rx` to `main_loop`.

## Verification & Testing
* **Build Check**: Ensure `cargo build` succeeds without lock contention warnings.
* **Unit Tests**: Ensure `cargo test` passes.
* **Integration Tests**: Run the VS Code integration tests via `just test vscode` to verify that code lenses and inlay hints update synchronously.
* **Concurrency Test**: Confirm that spamming document edits rapidly (which queue up `didChange` and `Parse` events) does not freeze the language server, verifying the removal of `RwLock` contention.