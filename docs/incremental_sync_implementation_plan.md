# Implementation Plan: Fix ts-j9r (Support multiple content changes in DidChangeTextDocument)

## Problem Description
Currently, the `DidChangeTextDocument` handler in `server.rs` only processes the *last* change in the `content_changes` array:
```rust
if let Some(change) = params.content_changes.into_iter().last() {
```
This restricts the server to `TextDocumentSyncKind::FULL`. If a client sends incremental changes (multiple edits with specific `Range` boundaries within a single notification), this code will drop all previous edits and treat the last partial edit text as the entire new document, resulting in catastrophic state corruption. 

## Discussion: INCREMENTAL vs. FULL Sync

Before proceeding, let's look at the trade-offs of switching from `FULL` to `INCREMENTAL` synchronization.

### Pros of INCREMENTAL Sync
1. **Network / IPC Bandwidth:** If the user is editing a 10,000-line file, `FULL` sync forces the client to send the entire file payload over standard I/O on *every single keystroke*. `INCREMENTAL` sync only sends the exact characters inserted/deleted.
2. **True $O(1)$ Coordinate Shifting:** With incremental changes, the client explicitly gives us the `Range` of the edit. We no longer need to use our string-diffing heuristic (`shift_results`) to discover the `pivot`! We can use the explicit `range` to shift coordinates precisely in constant time.
3. **Maturity:** Almost all enterprise-grade language servers use incremental sync for scalability.

### Cons / Counter-Arguments (Why stick with FULL Sync?)
1. **State Desynchronization Risks:** Splicing UTF-16 coordinate ranges into UTF-8 Rust `String`s is notoriously tricky. If the server misinterprets a range (e.g. splitting a surrogate pair or emoji), the server's document state becomes permanently corrupted and misaligned with the client until the file is reopened.
2. **Complexity:** Applying multiple patches sequentially requires careful reconstruction of the document and `LineIndex` at each step.
3. **It Might Be Unnecessary:** With our recent lazy evaluation optimization (`ts-f49`), `FULL` sync coordinate shifting is already practically $O(1)$ for 99% of typing scenarios. Unless users are writing absolutely massive Scheme files, the IPC bandwidth of `FULL` sync over local stdio is unlikely to be a real bottleneck. 

> [!IMPORTANT]
> If we stick with `FULL` sync, the current implementation (`params.content_changes.into_iter().last()`) is actually **100% correct** according to the LSP specification. When using `FULL` sync, the client sends exactly one change containing the full document, or in the rare case it batches them, the last one represents the current final state.
> 
> Therefore, if we decide the risks of `INCREMENTAL` outweigh the benefits, we should simply close `ts-j9r` as "Won't Fix / Works as Intended."

## Proposed Solution (If we choose to proceed)

To support `TextDocumentSyncKind::INCREMENTAL` and multiple content changes in a single notification, we must apply each edit to the document sequentially.

### 1. Update Server Capabilities
In `lsp/src/main.rs`, switch the `text_document_sync` capability from `FULL` to `INCREMENTAL`.

### 2. Iterative Application of Changes
In `lsp/src/server.rs`'s `DidChangeTextDocument` handler, iterate over *all* `change` entries in `params.content_changes`.
For each change:
- If `change.range` is `Some(range)`, calculate the start and end byte indices using the existing `doc.line_index`, and use `String::replace_range` to splice the new text into `doc.text`.
- If `change.range` is `None`, treat it as a full document replacement (this is standard LSP fallback behavior).
- Call `shift_results` on the intermediate state.
- Update the document text and line index.

### 3. TDD Strategy
Before modifying the server logic, I will add an integration test (`test_incremental_did_change` in `integration.rs`) that:
1. Opens a file and evaluates it (to generate results).
2. Sends a `didChange` notification with *two* simultaneous incremental edits.
3. Queries `inlayHint` to verify that `shift_results` accurately shifted the coordinates of the evaluated results based on both incremental changes.

## Verification Plan
- The new `test_incremental_did_change` must fail initially (since the server drops the first change).
- The test must pass after the fix.
- All existing tests (which use `FULL` sync without ranges) must still pass, ensuring backward compatibility.

## User Review Required
Please review this approach. If approved, I will begin by writing the failing test and then implementing the iterative change logic!
