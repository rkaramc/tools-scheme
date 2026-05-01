use std::sync::{Arc, RwLock};
use std::sync::atomic::AtomicI32;
use std::str::FromStr;
use url::Url;
use lsp_server::{Message, Request, RequestId};
use lsp_types::{
    notification::{Notification as _, PublishDiagnostics},
    Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range,
};
use crate::server::{SharedState, RwLockExt};
use crate::evaluator::{EvalResult, Evaluator};
use crate::coordinates::LineIndex;
use crate::documents::DocumentSnapshot;

static NEXT_REQ_ID: AtomicI32 = AtomicI32::new(100);

pub trait MessageSender {
    fn send_diagnostics(&self, uri: lsp_types::Uri, diagnostics: Vec<Diagnostic>, version: Option<i32>);
    fn refresh_inlay_hints(&self);
    fn refresh_code_lenses(&self);
    fn send_notification(&self, method: String, params: serde_json::Value);
}

pub struct DiagnosticTask {
    pub uri: lsp_types::Uri,
    pub diagnostics: Vec<Diagnostic>,
    pub version: Option<i32>,
}

pub struct DiagnosticWorkerSender {
    pub lsp_sender: crossbeam_channel::Sender<Message>,
    pub diagnostic_tx: crossbeam_channel::Sender<DiagnosticTask>,
}

impl MessageSender for DiagnosticWorkerSender {
    fn send_diagnostics(&self, uri: lsp_types::Uri, diagnostics: Vec<Diagnostic>, version: Option<i32>) {
        let _ = self.diagnostic_tx.send(DiagnosticTask { uri, diagnostics, version });
    }

    fn refresh_inlay_hints(&self) {
        let id = NEXT_REQ_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = Request::new(RequestId::from(id), "workspace/inlayHint/refresh".to_string(), serde_json::json!(null));
        let _ = self.lsp_sender.send(Message::Request(req));
    }

    fn refresh_code_lenses(&self) {
        let id = NEXT_REQ_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = Request::new(RequestId::from(id), "workspace/codeLens/refresh".to_string(), serde_json::json!(null));
        let _ = self.lsp_sender.send(Message::Request(req));
    }

    fn send_notification(&self, method: String, params: serde_json::Value) {
        let not = lsp_server::Notification::new(method, params);
        let _ = self.lsp_sender.send(Message::Notification(not));
    }
}

pub fn diagnostic_worker(
    rx: crossbeam_channel::Receiver<DiagnosticTask>,
    sender: crossbeam_channel::Sender<Message>,
) {
    let mut pending: std::collections::HashMap<lsp_types::Uri, DiagnosticTask> = std::collections::HashMap::new();
    let is_test = std::env::var("TOOLS_SCHEME_TEST").is_ok();
    let debounce_ms = if is_test { 0 } else { 200 };
    let max_debounce_ms = 1000; // Hard cap of 1 second for any burst

    loop {
        match rx.recv() {
            Ok(task) => {
                let burst_start = std::time::Instant::now();
                pending.insert(task.uri.clone(), task);
                
                // Keep collecting as long as they come in fast, but don't wait forever
                if debounce_ms > 0 {
                    while let Ok(task) = rx.recv_timeout(std::time::Duration::from_millis(debounce_ms)) {
                        pending.insert(task.uri.clone(), task);
                        if burst_start.elapsed().as_millis() >= max_debounce_ms as u128 {
                            break;
                        }
                    }
                }
                
                // Silence reached, max delay hit, or immediate mode (debounce_ms == 0), flush all
                for (_, task) in pending.drain() {
                    let diag_params = PublishDiagnosticsParams { 
                        uri: task.uri, 
                        diagnostics: task.diagnostics, 
                        version: task.version 
                    };
                    let not = lsp_server::Notification::new(PublishDiagnostics::METHOD.to_string(), diag_params);
                    let _ = sender.send(Message::Notification(not));
                }
            }
            Err(_) => break, // Channel closed
        }
    }
}

impl MessageSender for crossbeam_channel::Sender<Message> {
    fn send_diagnostics(&self, uri: lsp_types::Uri, diagnostics: Vec<Diagnostic>, version: Option<i32>) {
        let diag_params = PublishDiagnosticsParams { uri, diagnostics, version };
        let not = lsp_server::Notification::new(PublishDiagnostics::METHOD.to_string(), diag_params);
        let _ = self.send(Message::Notification(not));
    }

    fn refresh_inlay_hints(&self) {
        let id = NEXT_REQ_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = Request::new(RequestId::from(id), "workspace/inlayHint/refresh".to_string(), serde_json::json!(null));
        let _ = self.send(Message::Request(req));
    }

    fn refresh_code_lenses(&self) {
        let id = NEXT_REQ_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let req = Request::new(RequestId::from(id), "workspace/codeLens/refresh".to_string(), serde_json::json!(null));
        let _ = self.send(Message::Request(req));
    }

    fn send_notification(&self, method: String, params: serde_json::Value) {
        let not = lsp_server::Notification::new(method, params);
        let _ = self.send(Message::Notification(not));
    }
}

pub enum EvalAction {
    Evaluate { 
        snapshot: Option<DocumentSnapshot>,
        content: String, 
        request_id: RequestId, 
        version: Option<i32>, 
        offset: Option<(u32, u32)>, 
        byte_range: Option<(u32, u32)> 
    },
    Parse { snapshot: Option<DocumentSnapshot>, version: i32 },
    Clear,
    Restart,
    EvalCell { 
        snapshot: Option<DocumentSnapshot>,
        code: String, 
        execution_id: u32,
        notebook_uri: Option<String>,
        version: Option<i32>,
    },
}

pub struct EvalTask {
    pub uri: String,
    pub action: EvalAction,
}

pub fn eval_worker(
    mut evaluator: Evaluator,
    rx: crossbeam_channel::Receiver<EvalTask>,
    cancel_rx: crossbeam_channel::Receiver<u32>,
    state: Arc<RwLock<SharedState>>,
    sender: impl MessageSender,
) {
    for task in rx {
        match task.action {
            EvalAction::Evaluate { snapshot, content, version, offset, byte_range, request_id } => {
                on_evaluate(&mut evaluator, &state, &sender, &task.uri, snapshot, content, version, offset, byte_range, request_id);
            }
            EvalAction::Clear => {
                on_clear(&mut evaluator, &state, &sender, &task.uri);
            }
            EvalAction::Restart => {
                on_restart(&mut evaluator, &state, &sender);
            }
            EvalAction::EvalCell { snapshot, code, execution_id, notebook_uri, version } => {
                on_eval_cell(&mut evaluator, &state, &sender, &cancel_rx, &task.uri, snapshot, notebook_uri, code, execution_id, version);
            }
            EvalAction::Parse { .. } => {
                evaluator.log("WARNING: EvalWorker received Parse action. This should be routed to AnalysisActor.");
            }
        }
    }
    evaluator.log("Eval worker channel closed, shutting down evaluator");
    evaluator.shutdown();
}

pub fn analysis_worker(
    mut evaluator: Evaluator,
    rx: crossbeam_channel::Receiver<EvalTask>,
    state: Arc<RwLock<SharedState>>,
    sender: impl MessageSender,
) {
    for task in rx {
        match task.action {
            EvalAction::Parse { snapshot, version } => {
                // eprintln!("AnalysisWorker: Received Parse task for {} version {}", task.uri, version);
                on_parse(&mut evaluator, &state, &sender, &task.uri, snapshot, version);
            }
            EvalAction::Restart => {
                // We only need to restart the evaluator process. We shouldn't clear the state here
                // because eval_worker handles state clearing to avoid race conditions.
                evaluator.log("AnalysisWorker: Restart triggered");
                let _ = evaluator.restart();
            }
            _ => {
                evaluator.log(&format!("WARNING: AnalysisWorker received non-analysis action for uri: {}", task.uri));
            }
        }
    }
    evaluator.log("Analysis worker channel closed, shutting down evaluator");
    evaluator.shutdown();
}

#[allow(clippy::too_many_arguments)]
fn on_evaluate(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &impl MessageSender,
    uri_str: &str,
    snapshot: Option<DocumentSnapshot>,
    content: String,
    version: Option<i32>,
    offset: Option<(u32, u32)>,
    byte_range: Option<(u32, u32)>,
    request_id: RequestId,
) {
    let uri = match lsp_types::Uri::from_str(uri_str) {
        Ok(u) => u,
        Err(_) => return,
    };
    let context_label = Url::parse(uri.as_str()).ok()
        .and_then(|u| u.to_file_path().ok())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));

    let log_handle = {
        let lock = state.read_recovered();
        if let Some(doc) = lock.document_store.get(uri_str) {
            if doc.version > version.unwrap_or(0) {
                evaluator.log(&format!("Pre-flight check failed: doc version {} > task version {:?}", doc.version, version));
                return;
            }
            doc.session_file.as_ref().and_then(|f| f.try_clone().ok())
        } else {
            None
        }
    };

    evaluator.log(&format!("EvalAction::Evaluate(content, version: {:?}, offset: {:?}, byte_range: {:?}, request_id: {:?})", version, offset, byte_range, request_id));

    let eval_results = evaluator.evaluate_str(&content, Some(uri_str), context_label.as_deref(), log_handle.as_ref());

    match eval_results {
        Ok(mut results) => {
            // Convert Racket's character offsets to byte offsets relative to the evaluated content.
            for res in &mut results {
                let char_idx = res.pos.saturating_sub(1) as usize;
                let byte_off: usize = crate::coordinates::RacketCharIndices::new(&content)
                    .take(char_idx)
                    .map(|(_, s)| s.len())
                    .sum();
                res.pos = (byte_off + 1) as u32;
            }

            // Adjust byte pos based on selection offset
            let is_selection = offset.is_some();
            if is_selection {
                let start_byte_off = byte_range.map(|(s, _)| s).unwrap_or(0);
                for res in &mut results {
                    res.pos += start_byte_off;
                }
            }

            // Normalize coordinates and build diagnostics.
            let mut final_diagnostics = Vec::new();
            let is_stale = {
                let mut lock = state.write_recovered();
                if let Some(doc) = lock.document_store.get_mut(uri_str) {
                    if doc.version > version.unwrap_or(0) {
                        evaluator.log(&format!("Post-flight check failed: doc version {} > task version {:?}", doc.version, version));
                        true
                    } else {
                        // Use snapshot for normalization if available, otherwise use current document text/index
                        let (norm_text, norm_index) = if let Some(snap) = &snapshot {
                            (&*snap.text, &*snap.line_index)
                        } else {
                            (&*doc.text, &*doc.line_index)
                        };

                        if is_selection {
                            recalculate_from_byte_pos(&mut results, norm_text, norm_index);
                        } else {
                            normalize_results(&mut results, norm_text, norm_index);
                        }

                        merge_results(&mut doc.results, results, byte_range);
                        
                        // Build diagnostics from the COMPOSITE results after merge
                        final_diagnostics = doc.results
                            .iter()
                            .filter(|r| r.is_error)
                            .map(|res| {
                                let range = Range::new(
                                    Position::new(res.line.saturating_sub(1), res.col),
                                    Position::new(res.end_line.saturating_sub(1), res.end_col),
                                );
                                
                                let mut severity = DiagnosticSeverity::ERROR;
                                if uri_str.starts_with("vscode-notebook-cell:") {
                                    let msg_lower = res.result.to_lowercase();
                                    if msg_lower.contains("duplicate identifier") || msg_lower.contains("duplicate binding") {
                                        severity = DiagnosticSeverity::WARNING;
                                    }
                                }

                                Diagnostic {
                                    range,
                                    severity: Some(severity),
                                    message: res.result.clone(),
                                    ..Default::default()
                                }
                            })
                            .collect();
                        false
                    }
                } else {
                    // Fallback for one-off evaluations (no document in store)
                    if let Some(snap) = &snapshot {
                        if is_selection {
                            recalculate_from_byte_pos(&mut results, &snap.text, &snap.line_index);
                        } else {
                            normalize_results(&mut results, &snap.text, &snap.line_index);
                        }
                    } else {
                        let temp_idx = crate::coordinates::LineIndex::new(&content);
                        if is_selection {
                            recalculate_from_byte_pos(&mut results, &content, &temp_idx);
                        } else {
                            normalize_results(&mut results, &content, &temp_idx);
                        }
                    };

                    final_diagnostics = results
                        .iter()
                        .filter(|r| r.is_error)
                        .map(|res| {
                            let range = Range::new(
                                Position::new(res.line.saturating_sub(1), res.col),
                                Position::new(res.end_line.saturating_sub(1), res.end_col),
                            );

                            let mut severity = DiagnosticSeverity::ERROR;
                            if uri_str.starts_with("vscode-notebook-cell:") {
                                let msg_lower = res.result.to_lowercase();
                                if msg_lower.contains("duplicate identifier") || msg_lower.contains("duplicate binding") {
                                    severity = DiagnosticSeverity::WARNING;
                                }
                            }

                            Diagnostic {
                                range,
                                severity: Some(severity),
                                message: res.result.clone(),
                                ..Default::default()
                            }
                        })
                        .collect();
                    false
                }
            };

            if is_stale { return; }

            // Publish composite diagnostics.
            sender.send_diagnostics(uri, final_diagnostics, version);

            // Ask the client to refresh inlay hints.
            sender.refresh_inlay_hints();
        }
        Err(e) => {
            let diagnostics = vec![Diagnostic {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                severity: Some(DiagnosticSeverity::ERROR),
                message: format!("Evaluation error: {}", e),
                ..Default::default()
            }];
            sender.send_diagnostics(uri, diagnostics, version);
        }
    }
}

fn on_parse(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &impl MessageSender,
    uri_str: &str,
    snapshot: Option<DocumentSnapshot>,
    version: i32,
) {
    evaluator.log(&format!("EvalAction::Parse(version: {:?}) for {}", version, uri_str));
    
    let (content, current_version) = if let Some(snap) = &snapshot {
        (Some(Arc::clone(&snap.text)), Some(snap.version))
    } else {
        let lock = state.read_recovered();
        if let Some(doc) = lock.document_store.get(uri_str) {
            (Some(Arc::clone(&doc.text)), Some(doc.version))
        } else {
            (None, None)
        }
    };

    if let (Some(c), Some(v)) = (content, current_version) {
        if v > version {
            evaluator.log("Skipping parse: newer version already in store");
            return;
        }

        // Use unified parser from Racket shim
        let ranges = match evaluator.parse(&c, Some(uri_str)) {
            Ok(r) => r,
            Err(e) => {
                evaluator.log(&format!("Parse error: {}", e));
                return;
            }
        };

        let is_stale = {
            let mut lock = state.write_recovered();
            if let Some(doc) = lock.document_store.get_mut(uri_str) {
                if doc.version > version {
                    evaluator.log(&format!("Post-flight check failed in Parse: doc version {} > task version {:?}", doc.version, version));
                    true
                } else {
                    let mut final_ranges = Vec::new();
                    for r in ranges {
                        // Only show CodeLens for code blocks that are syntactically valid
                        if r.kind == "code" && r.valid {
                            let start_offset = (r.pos.saturating_sub(1)) as usize;
                            let end_offset = start_offset + (r.span as usize);
                            let range = doc.line_index.offset_to_range(&c, start_offset, end_offset);
                            final_ranges.push(range);
                        }
                    }

                    doc.ranges = final_ranges;
                    evaluator.log(&format!("Assigned {} valid ranges", doc.ranges.len()));
                    false
                }
            } else {
                false
            }
        };

        if is_stale { return; }

        // Ask the client to refresh code lenses
        sender.refresh_code_lenses();
    }
}

fn on_clear(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &impl MessageSender,
    uri_str: &str,
) {
    {
        let lock = state.read_recovered();
        let log = lock.document_store.get(uri_str).and_then(|doc| doc.session_file.as_ref());
        let _ = evaluator.clear_namespace(uri_str, log);
    }
    let mut lock = state.write_recovered();
    if let Some(doc) = lock.document_store.get_mut(uri_str) {
        doc.results.clear();
        evaluator.log(&format!("Cleared results for {}", uri_str));
    } else {
        evaluator.log(&format!("Document not found for clear: {}", uri_str));
    }

    evaluator.log("Namespace cleared, sending refreshes");
    
    // Trigger refreshes
    sender.refresh_inlay_hints();
    sender.refresh_code_lenses();
}

fn on_restart(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &impl MessageSender,
) {
    evaluator.log("EvalAction::Restart triggered");
    let _ = evaluator.restart();
    
    let uris: Vec<String> = {
        let mut lock = state.write_recovered();
        let uris: Vec<String> = lock.document_store.iter().map(|(uri, _)| uri.clone()).collect();
        for doc in lock.document_store.iter_mut() {
            doc.results.clear();
            doc.ranges.clear();
        }
        uris
    };
    
    evaluator.log(&format!("Restart cleared state for {} documents", uris.len()));
    
    // Trigger refreshes for all documents
    for uri_str in uris {
        if let Ok(uri) = lsp_types::Uri::from_str(&uri_str) {
            evaluator.log(&format!("Clearing diagnostics for {}", uri_str));
            sender.send_diagnostics(uri, Vec::new(), None);
        }
    }

    sender.refresh_inlay_hints();
    sender.refresh_code_lenses();
}


fn merge_results(existing: &mut Vec<EvalResult>, new_results: Vec<EvalResult>, byte_range: Option<(u32, u32)>) {
    if let Some((start, end)) = byte_range {
        existing.retain(|res| {
            let zero_indexed_pos = res.pos.saturating_sub(1);
            zero_indexed_pos < start || zero_indexed_pos >= end
        });
        existing.extend(new_results);
    } else {
        *existing = new_results;
    }
}

fn normalize_results(results: &mut [EvalResult], text: &str, line_index: &LineIndex) {
    for res in results.iter_mut() {
        let start_line_idx = res.line.saturating_sub(1) as usize;
        let start_col_idx = res.col as usize;
        let pos_byte_idx = line_index.byte_offset(text, start_line_idx, start_col_idx, crate::coordinates::OffsetUnit::CodePoint);
        res.pos = (pos_byte_idx + 1) as u32;
    }
    recalculate_from_byte_pos(results, text, line_index);
}

pub fn recalculate_from_byte_pos(results: &mut [EvalResult], text: &str, line_index: &LineIndex) {
    for res in results.iter_mut() {
        let pos_byte_idx = res.pos.saturating_sub(1) as usize;
        let mut end_byte_idx = pos_byte_idx;
        let mut chars = crate::coordinates::RacketCharIndices::new(&text[pos_byte_idx.min(text.len())..]);
        for _ in 0..res.span {
            if let Some((_, s)) = chars.next() {
                end_byte_idx += s.len();
            } else {
                break;
            }
        }
        let start_pos = line_index.offset_to_position(text, pos_byte_idx);
        let end_pos = line_index.offset_to_position(text, end_byte_idx);
        res.line = start_pos.line + 1;
        res.col = start_pos.character;
        res.end_line = end_pos.line + 1;
        res.end_col = end_pos.character;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::EvalResult;
    use std::time::Duration;

    fn make_res(pos: u32, val: &str) -> EvalResult {
        EvalResult {
            line: 1, col: 0, end_line: 1, end_col: 0, span: 0,
            pos, result: val.to_string(), is_error: false, output: "".to_string(),
            kind: "code".to_string()
        }
    }

    #[test]
    fn test_diagnostic_worker_debounce() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (lsp_tx, lsp_rx) = crossbeam_channel::unbounded();
        
        let _handle = std::thread::spawn(move || {
            diagnostic_worker(rx, lsp_tx);
        });

        let uri = lsp_types::Uri::from_str("file:///test.rkt").unwrap();
        
        // Send 3 tasks quickly
        tx.send(DiagnosticTask { 
            uri: uri.clone(), 
            diagnostics: vec![Diagnostic { message: "error 1".to_string(), ..Default::default() }],
            version: Some(1)
        }).unwrap();
        tx.send(DiagnosticTask { 
            uri: uri.clone(), 
            diagnostics: vec![Diagnostic { message: "error 2".to_string(), ..Default::default() }],
            version: Some(2)
        }).unwrap();
        tx.send(DiagnosticTask { 
            uri: uri.clone(), 
            diagnostics: vec![Diagnostic { message: "error 3".to_string(), ..Default::default() }],
            version: Some(3)
        }).unwrap();

        // Wait for it to flush (debounce is 200ms)
        std::thread::sleep(Duration::from_millis(500));
        
        // Should only get ONE message (the last one)
        let msg = lsp_rx.recv_timeout(Duration::from_millis(100)).expect("Should have received a message");
        if let Message::Notification(not) = msg {
            assert_eq!(not.method, PublishDiagnostics::METHOD);
            let params: PublishDiagnosticsParams = serde_json::from_value(not.params).unwrap();
            assert_eq!(params.diagnostics.len(), 1);
            assert_eq!(params.diagnostics[0].message, "error 3");
            assert_eq!(params.version, Some(3));
        } else {
            panic!("Expected notification");
        }

        // Should NOT have any more messages
        assert!(lsp_rx.try_recv().is_err(), "Should have only received one debounced message");
    }

    #[test]
    fn test_diagnostic_worker_infinite_debounce_risk() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (lsp_tx, lsp_rx) = crossbeam_channel::unbounded();
        
        let _handle = std::thread::spawn(move || {
            diagnostic_worker(rx, lsp_tx);
        });

        let uri = lsp_types::Uri::from_str("file:///test.rkt").unwrap();
        
        // Send 10 tasks with 100ms interval. Debounce is 200ms.
        // If it's buggy, it will never flush during the 1 second.
        for i in 1..=10 {
            tx.send(DiagnosticTask {
                uri: uri.clone(),
                diagnostics: vec![Diagnostic { message: format!("error {}", i), ..Default::default() }],
                version: Some(i)
            }).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            
            // It should NOT have flushed yet because the burst is ongoing
            assert!(lsp_rx.try_recv().is_err(), "Flushed prematurely during burst at task {}", i);
        }

        // Wait another 500ms to be sure it flushes after the burst.
        std::thread::sleep(Duration::from_millis(500));
        
        let mut count = 0;
        while let Ok(_) = lsp_rx.try_recv() {
            count += 1;
        }
        assert_eq!(count, 1, "Should have received exactly one debounced message after burst");
    }

    #[test]
    fn test_diagnostic_worker_max_debounce() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (lsp_tx, lsp_rx) = crossbeam_channel::unbounded();
        
        let _handle = std::thread::spawn(move || {
            diagnostic_worker(rx, lsp_tx);
        });

        let uri = lsp_types::Uri::from_str("file:///test.rkt").unwrap();
        
        // Send tasks with 100ms interval for 2 seconds.
        // max_debounce_ms is 1000ms.
        // We expect at least one flush around 1 second.
        let start = std::time::Instant::now();
        let mut flushed_at_least_once = false;
        for i in 1..=20 {
            tx.send(DiagnosticTask {
                uri: uri.clone(),
                diagnostics: vec![Diagnostic { message: format!("error {}", i), ..Default::default() }],
                version: Some(i)
            }).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            
            if lsp_rx.try_recv().is_ok() {
                flushed_at_least_once = true;
                let elapsed = start.elapsed().as_millis();
                assert!(elapsed >= 800 && elapsed <= 1500, "Flush happened too early or too late: {}ms", elapsed);
            }
        }

        assert!(flushed_at_least_once, "Should have flushed during the long burst due to max_debounce_ms");
    }

    #[test]
    fn test_merge_results() {
        let mut existing = vec![
            make_res(10, "old1"),
            make_res(20, "old2"),
            make_res(30, "old3"),
        ];

        let new_results = vec![make_res(20, "new2")];
        
        // Merge with range covering old2 [15, 25]
        // old2 pos=20. 20-1=19. 19 is in [15, 25].
        merge_results(&mut existing, new_results, Some((15, 25)));

        assert_eq!(existing.len(), 3);
        assert_eq!(existing[0].result, "old1");
        assert_eq!(existing[1].result, "old3");
        assert_eq!(existing[2].result, "new2");

        // Edge case: pos=10. 10-1=9. Range [10, 20]. 
        // 9 < 10. Should NOT be cleared.
        let mut existing = vec![make_res(10, "old")];
        merge_results(&mut existing, vec![], Some((10, 20)));
        assert_eq!(existing.len(), 1, "Boundary case: pos=10 (idx 9) should not be cleared by range [10, 20]");

        // Edge case: pos=11. 11-1=10. Range [10, 20].
        // 10 >= 10. Should BE cleared.
        let mut existing = vec![make_res(11, "old")];
        merge_results(&mut existing, vec![], Some((10, 20)));
        assert_eq!(existing.len(), 0, "Boundary case: pos=11 (idx 10) should be cleared by range [10, 20]");
        
        // Full overwrite
        merge_results(&mut existing, vec![make_res(50, "final")], None);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].result, "final");
    }
}

fn on_eval_cell(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &impl MessageSender,
    cancel_rx: &crossbeam_channel::Receiver<u32>,
    uri_str: &str,
    _snapshot: Option<DocumentSnapshot>,
    notebook_uri: Option<String>,
    code: String,
    execution_id: u32,
    version: Option<i32>,
) {
    let uri = match lsp_types::Uri::from_str(uri_str) {
        Ok(u) => u,
        Err(_) => return,
    };
    let eval_uri = notebook_uri.as_deref().unwrap_or(uri_str);
    
    let mut diagnostics = Vec::new();

    let result = evaluator.evaluate_notebook_cell(&code, eval_uri, cancel_rx, execution_id, |line| {
        // Parse the line from evaluator. It might be {"type":"output",...}, {"type":"rich",...}, or {"type":"range",...} containing "result"
        if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(line) {
            let output_type = json_val.get("type").and_then(|v| v.as_str());
            
            if output_type == Some("output") {
                let stream = json_val.get("stream").and_then(|v| v.as_str()).unwrap_or("stdout");
                let data = json_val.get("data").and_then(|v| v.as_str()).unwrap_or("");
                let params = serde_json::json!({
                    "executionId": execution_id,
                    "payload": {
                        "type": stream,
                        "data": data
                    }
                });
                sender.send_notification("scheme/notebook/outputStream".to_string(), params);
            } else if output_type == Some("rich") {
                let mime = json_val.get("mime").and_then(|v| v.as_str()).unwrap_or("image/png");
                let data = json_val.get("data").and_then(|v| v.as_str()).unwrap_or("");
                let params = serde_json::json!({
                    "executionId": execution_id,
                    "payload": {
                        "type": "rich",
                        "mime": mime,
                        "data": data
                    }
                });
                sender.send_notification("scheme/notebook/outputStream".to_string(), params);
            } else if let Some(res_str) = json_val.get("result").and_then(|v| v.as_str()) {
                // It's a standard result or evaluation error reported via display-result
                let is_error = json_val.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                let typ = if is_error { "error" } else { "result" };
                
                let params = serde_json::json!({
                    "executionId": execution_id,
                    "payload": {
                        "type": typ,
                        "data": res_str
                    }
                });
                sender.send_notification("scheme/notebook/outputStream".to_string(), params);

                if is_error {
                    let line = json_val.get("line").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                    let col = json_val.get("col").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let end_line = json_val.get("end_line").and_then(|v| v.as_u64()).unwrap_or(line as u64) as u32;
                    let end_col = json_val.get("end_col").and_then(|v| v.as_u64()).unwrap_or(col as u64) as u32;

                    let range = Range::new(
                        Position::new(line.saturating_sub(1), col),
                        Position::new(end_line.saturating_sub(1), end_col),
                    );
                    
                    let mut severity = DiagnosticSeverity::ERROR;
                    let msg_lower = res_str.to_lowercase();
                    if uri_str.starts_with("vscode-notebook-cell:") {
                        if msg_lower.contains("duplicate identifier") || msg_lower.contains("duplicate binding") {
                            severity = DiagnosticSeverity::WARNING;
                        }
                    }

                    diagnostics.push(Diagnostic {
                        range,
                        severity: Some(severity),
                        message: res_str.to_string(),
                        ..Default::default()
                    });
                }
            }
        }
    });

    // Check post-flight version before sending diagnostics
    let is_stale = {
        let lock = state.read_recovered();
        if let Some(doc) = lock.document_store.get(uri_str) {
            doc.version > version.unwrap_or(0)
        } else {
            false
        }
    };

    if is_stale {
        evaluator.log(&format!("Post-flight check failed in EvalCell: document version > task version {:?}", version));
    } else {
        // Send collected diagnostics for the cell
        sender.send_diagnostics(uri, diagnostics, version);
    }

    let eval_finished_params = match result {
        Ok(_) => serde_json::json!({
            "executionId": execution_id,
            "success": true
        }),
        Err(e) => {
            // Send error to outputStream
            let err_params = serde_json::json!({
                "executionId": execution_id,
                "payload": {
                    "type": "error",
                    "data": format!("Evaluation failed: {}", e)
                }
            });
            sender.send_notification("scheme/notebook/outputStream".to_string(), err_params);
            
            serde_json::json!({
                "executionId": execution_id,
                "success": false
            })
        }
    };

    sender.send_notification("scheme/notebook/evalFinished".to_string(), eval_finished_params);
}

