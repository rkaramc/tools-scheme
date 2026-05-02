use std::sync::atomic::AtomicI32;
use url::Url;
use lsp_server::{Message, Request, RequestId};
use lsp_types::{
    notification::{Notification as _, PublishDiagnostics},
    Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range,
};
use crate::server::WorkerResult;
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
    disable_diagnostics: bool,
) {
    if disable_diagnostics {
        for _ in rx {}
        return;
    }

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
    PullRichMedia {
        id: String,
        request_id: RequestId,
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
    result_tx: crossbeam_channel::Sender<WorkerResult>,
    sender: impl MessageSender,
) {
    for task in rx {
        match task.action {
            EvalAction::Evaluate { snapshot, content, version, offset, byte_range, request_id } => {
                on_evaluate(&mut evaluator, &result_tx, &sender, &task.uri, snapshot, content, version, offset, byte_range, request_id);
            }
            EvalAction::Clear => {
                on_clear(&mut evaluator, &result_tx, &task.uri);
            }
            EvalAction::Restart => {
                on_restart(&mut evaluator, &result_tx);
            }
            EvalAction::EvalCell { snapshot, code, execution_id, notebook_uri, version } => {
                on_eval_cell(&mut evaluator, &result_tx, &sender, &cancel_rx, &task.uri, snapshot, notebook_uri, code, execution_id, version);
            }
            EvalAction::PullRichMedia { id, request_id } => {
                on_pull_rich_media(&mut evaluator, &result_tx, &task.uri, id, request_id);
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
    result_tx: crossbeam_channel::Sender<WorkerResult>,
    _sender: impl MessageSender,
) {

    for task in rx {
        match task.action {
            EvalAction::Parse { snapshot, version } => {
                on_parse(&mut evaluator, &result_tx, &task.uri, snapshot, version);
            }
            EvalAction::Restart => {
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
    result_tx: &crossbeam_channel::Sender<WorkerResult>,
    _sender: &impl MessageSender,
    uri_str: &str,
    snapshot: Option<DocumentSnapshot>,
    content: String,
    version: Option<i32>,
    offset: Option<(u32, u32)>,
    byte_range: Option<(u32, u32)>,
    request_id: RequestId,
) {
    let context_label = Url::parse(uri_str).ok()
        .and_then(|u| u.to_file_path().ok())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));

    let log_handle = snapshot.as_ref().and_then(|snap| snap.session_file.as_ref().as_ref()).and_then(|f| f.try_clone().ok());

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

            // Use snapshot for normalization
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

            let _ = result_tx.send(WorkerResult::EvaluateComplete {
                uri: uri_str.to_string(),
                version,
                results,
                byte_range,
            });
        }
        Err(e) => {
            let diagnostics = vec![Diagnostic {
                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                severity: Some(DiagnosticSeverity::ERROR),
                message: format!("Evaluation error: {}", e),
                ..Default::default()
            }];
            let _ = result_tx.send(WorkerResult::EvaluationError {
                uri: uri_str.to_string(),
                version,
                diagnostics,
            });
        }
    }
}

fn on_parse(
    evaluator: &mut Evaluator,
    result_tx: &crossbeam_channel::Sender<WorkerResult>,
    uri_str: &str,
    snapshot: Option<DocumentSnapshot>,
    version: i32,
) {
    if let Some(snap) = &snapshot {
        // Use unified parser from Racket shim
        let ranges = match evaluator.parse(&snap.text, Some(uri_str)) {
            Ok(r) => r,
            Err(e) => {
                evaluator.log(&format!("Parse error: {}", e));
                return;
            }
        };

        let mut final_ranges = Vec::new();
        for r in ranges {
            // Only show CodeLens for code blocks that are syntactically valid
            if r.kind == "code" && r.valid {
                let start_offset = (r.pos.saturating_sub(1)) as usize;
                let end_offset = start_offset + (r.span as usize);
                let range = snap.line_index.offset_to_range(&snap.text, start_offset, end_offset);
                final_ranges.push(range);
            }
        }

        let _ = result_tx.send(WorkerResult::ParseComplete {
            uri: uri_str.to_string(),
            version,
            ranges: final_ranges,
        });
    }
}

fn on_clear(
    evaluator: &mut Evaluator,
    result_tx: &crossbeam_channel::Sender<WorkerResult>,
    uri_str: &str,
) {
    let _ = evaluator.clear_namespace(uri_str, None);
    let _ = result_tx.send(WorkerResult::ClearNamespace { uri: uri_str.to_string() });
}

fn on_restart(
    evaluator: &mut Evaluator,
    result_tx: &crossbeam_channel::Sender<WorkerResult>,
) {
    evaluator.log("EvalAction::Restart triggered");
    let _ = evaluator.restart();
    let _ = result_tx.send(WorkerResult::RestartComplete); // Gateway will clear all
}

fn on_pull_rich_media(
    evaluator: &mut Evaluator,
    result_tx: &crossbeam_channel::Sender<WorkerResult>,
    _uri_str: &str,
    id: String,
    request_id: RequestId,
) {
    match evaluator.get_rich_media(&id, None) {
        Ok(data) => {
            let _ = result_tx.send(WorkerResult::RichMedia { id, data, request_id });
        }
        Err(e) => {
            evaluator.log(&format!("Error pulling rich media {}: {}", id, e));
            let _ = result_tx.send(WorkerResult::RichMedia { id, data: String::new(), request_id });
        }
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
    use std::time::Duration;
    use std::str::FromStr;

    #[test]
    fn test_diagnostic_worker_disabled() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (lsp_tx, lsp_rx) = crossbeam_channel::unbounded();
        
        let _handle = std::thread::spawn(move || {
            diagnostic_worker(rx, lsp_tx, true);
        });

        let uri = lsp_types::Uri::from_str("file:///test.rkt").unwrap();
        
        tx.send(DiagnosticTask { 
            uri: uri.clone(), 
            diagnostics: vec![Diagnostic { message: "error 1".to_string(), ..Default::default() }],
            version: Some(1)
        }).unwrap();

        // Wait a bit
        std::thread::sleep(Duration::from_millis(100));
        
        // Should NOT have received any message
        assert!(lsp_rx.try_recv().is_err(), "Should NOT have received any message when disabled");
    }

    #[test]
    fn test_diagnostic_worker_debounce() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (lsp_tx, lsp_rx) = crossbeam_channel::unbounded();
        
        let _handle = std::thread::spawn(move || {
            diagnostic_worker(rx, lsp_tx, false);
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
            diagnostic_worker(rx, lsp_tx, false);
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
            diagnostic_worker(rx, lsp_tx, false);
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
}

fn on_eval_cell(
    evaluator: &mut Evaluator,
    result_tx: &crossbeam_channel::Sender<WorkerResult>,
    sender: &impl MessageSender,
    cancel_rx: &crossbeam_channel::Receiver<u32>,
    uri_str: &str,
    _snapshot: Option<DocumentSnapshot>,
    notebook_uri: Option<String>,
    code: String,
    execution_id: u32,
    version: Option<i32>,
) {
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
                let id = json_val.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let params = serde_json::json!({
                    "executionId": execution_id,
                    "payload": {
                        "type": "rich",
                        "mime": mime,
                        "id": id
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

    let _ = result_tx.send(WorkerResult::CellEvaluationComplete {
        uri: uri_str.to_string(),
        version,
        diagnostics,
    });

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

