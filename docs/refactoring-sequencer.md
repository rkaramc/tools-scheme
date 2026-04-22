# Refactoring Plan

Here is how I would sequence the remaining open issues. I've grouped them into three phases based on **context locality** (tackling related files together) and **dependency** (doing structural refactors before micro-optimizations).

## Phase 1: Document & Coordinate Stability

*Since we just worked on `shift_results` and `DocumentStore`, we should strike while the iron is hot and finish cleaning up this subsystem.*

- [x] **`ts-3is` — Refactor SharedState to move results and ranges into DocumentStore**
  -   **Why first:** This is a major structural change. It dictates *where* `shift_results` and `results` actually live. Doing this first prevents us from rewriting `shift_results` logic only to have to move it to a different file immediately after.

- [x] **`ts-f49` — Streamline `shift_results` logic**
  -   **Why second:** Once `shift_results` is safely encapsulated in `documents.rs` (thanks to step 1), we can confidently optimize its internal logic (removing the redundant $O(N)$ string equality checks) without interfering with `server.rs`.

- [x] **`ts-j9r` — Support multiple content changes in DidChangeTextDocument**
  -   **Why third:** This is a correctness bug that sits right on the boundary between receiving LSP notifications and updating the `DocumentStore`. With the store fully encapsulated, fixing how we loop through `content_changes` becomes much safer and cleaner.

## Phase 2: LSP Routing Cleanup
*These issues focus on cleaning up the massive `handle_request` and `handle_notification` match blocks in `server.rs`.*

- [x] **`ts-8f9` — Abstract boilerplate LSP message dispatching**

- [x] **`ts-3ad` — Use lsp-server dispatchers for routing**
  -   **Why here:** These two go hand-in-hand. The `lsp-server` crate provides built-in `RequestDispatcher` and `NotificationDispatcher` utilities. Adopting these will eliminate hundreds of lines of `if let Some(...) = cast_request(...)` boilerplate and dramatically shrink the main server loop.

- [x] **`ts-1p4` — Use enum-based Command Pattern for executeCommand**
  -   **Why here:** While `ts-3ad` cleaned up the top-level dispatchers, `handle_execute_command` still contains a monolithic `if/else` block and manual JSON indexing for custom Scheme commands. This refactor completes the routing cleanup by introducing a strongly-typed command enum.

## Phase 3: Concurrency & Worker Cleanup
*These issues deal with the background evaluation thread and lock safety.*

- [ ] **`ts-xag` — Break down eval_worker match block**
  -   **Why here:** Similar to the main loop, the background worker thread has a monolithic match block that handles parsing and evaluation. Breaking this into smaller functions makes the worker easier to maintain.

- [ ] **`ts-9rw` — Centralize state lock poison recovery**
  -   **Why last:** We saw this earlier (`.unwrap_or_else(|e| e.into_inner())`). This boilerplate is scattered everywhere. Once the dispatching is cleaned up (Phase 2) and the worker is refactored (Step 6), extracting this lock recovery into a centralized helper method will be the final polish on the server's state management.

***
