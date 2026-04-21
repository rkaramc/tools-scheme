use lsp_server::{Message, Request, RequestId, Response};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification,
        PublishDiagnostics,
    },
    request::{CodeActionRequest, CodeLensRequest, ExecuteCommand, InlayHintRequest}, 
    CodeActionOrCommand, CodeActionParams, CodeLens,
    CodeLensParams, Command, Diagnostic, DiagnosticSeverity, InlayHintParams,
    Position, PublishDiagnosticsParams, Range,
};
use serde_json::json;
use url::Url;
use std::{collections::HashMap, str::FromStr};
use std::error::Error;
use std::sync::{Arc, RwLock};
use crate::documents::DocumentStore;
use crate::evaluator::{EvalResult, Evaluator};
use crate::inlay_hints;
use crate::coordinates::LineIndex;

/// State shared between the main loop and the eval worker thread. 
pub struct SharedState {
    pub results: HashMap<String, Vec<EvalResult>>,
    pub ranges: HashMap<String, Vec<Range>>,
    pub document_store: DocumentStore,
}

pub enum EvalAction {
    Evaluate { content: String, request_id: RequestId, version: Option<i32>, offset: Option<(u32, u32)>, byte_range: Option<(u32, u32)> },
    Parse { version: i32 },
    Clear,
    Restart,
}

/// A request to perform an action in the evaluation worker thread.
pub struct EvalTask {
    pub uri: String,
    pub action: EvalAction,
}

pub struct Server {
    pub eval_tx: crossbeam_channel::Sender<EvalTask>,
    pub state: Arc<RwLock<SharedState>>,
}

/// Background thread: receives EvalTask, evaluates, updates SharedState, sends notifications.
pub fn eval_worker(
    mut evaluator: Evaluator,
    rx: crossbeam_channel::Receiver<EvalTask>,
    state: Arc<RwLock<SharedState>>,
    sender: crossbeam_channel::Sender<Message>,
) {
    for task in rx {
        match task.action {
            EvalAction::Evaluate { content, version, offset, byte_range, request_id } => {
                let uri_str = task.uri.clone();
                let uri = match lsp_types::Uri::from_str(&uri_str) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                let context_label = Url::parse(uri.as_str()).ok()
                    .and_then(|u| u.to_file_path().ok())
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));

                let log_handle = state.read().unwrap_or_else(|e| e.into_inner())
                    .document_store.get(&uri_str)
                    .and_then(|d| d.session_file.as_ref())
                    .and_then(|f| f.try_clone().ok());

                evaluator.log(&format!("EvalAction::Evaluate(content, version: {:?}, offset: {:?}, byte_range: {:?}, request_id: {:?})", version, offset, byte_range, request_id));

                let eval_results = evaluator.evaluate_str(&content, Some(&uri_str), context_label.as_deref(), log_handle.as_ref());

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
                                res.pos += start_byte_off as u32;
                            }
                        }

                        // Normalize coordinates to UTF-16 immediately using syntax-position and span.
                        if let Some(doc) = state.read().unwrap().document_store.get(&uri_str) {
                            if is_selection {
                                recalculate_from_byte_pos(&mut results, &doc.text, &doc.line_index);
                            } else {
                                normalize_results(&mut results, &doc.text, &doc.line_index);
                            }
                        }

                        // Build diagnostics while we still have the results in hand,
                        // before acquiring the write lock.
                        let diagnostics: Vec<Diagnostic> = {
                            let state_read = state.read().unwrap_or_else(|e| e.into_inner());
                            let _doc = state_read.document_store.get(&uri_str);
                            results
                                .iter()
                                .filter(|r| r.is_error)
                                        .map(|res| {
                                            let range = Range::new(
                                                Position::new(res.line.saturating_sub(1), res.col),
                                                Position::new(res.end_line.saturating_sub(1), res.end_col),
                                            );
                                            Diagnostic {
                                                range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                message: res.result.clone(),
                                                ..Default::default()
                                            }
                                        })
                                .collect()
                        };


                        // Store results with spatial merging.
                        {
                            let mut lock = state.write().unwrap_or_else(|e| e.into_inner());
                            let existing = lock.results.entry(uri_str.clone()).or_insert_with(Vec::new);
                            merge_results(existing, results, byte_range);
                        }

                        // Publish diagnostics.
                        let diag_params = PublishDiagnosticsParams {
                            uri: uri.clone(),
                            diagnostics,
                            version,
                        };
                        let not = lsp_server::Notification::new(
                            PublishDiagnostics::METHOD.to_string(),
                            diag_params,
                        );
                        let _ = sender.send(Message::Notification(not));

                        // Ask the client to refresh inlay hints.
                        let refresh_req = Request::new(
                            RequestId::from(999),
                            "workspace/inlayHint/refresh".to_string(),
                            json!(null),
                        );
                        let _ = sender.send(Message::Request(refresh_req));
                    }
                    Err(e) => {
                        // Send an error notification via diagnostics so the user sees it.
                        let diag_params = PublishDiagnosticsParams {
                            uri,
                            diagnostics: vec![Diagnostic {
                                range: Range::new(Position::new(0, 0), Position::new(0, 0)),
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: format!("Evaluation error: {}", e),
                                ..Default::default()
                            }],
                            version,
                        };
                        let not = lsp_server::Notification::new(
                            PublishDiagnostics::METHOD.to_string(),
                            diag_params,
                        );
                        let _ = sender.send(Message::Notification(not));
                    }
                }
            }
            EvalAction::Parse { version } => {
                evaluator.log(&format!("EvalAction::Parse(version: {:?}) for {}", version, task.uri));
                let (content, current_version) = {
                    let lock = state.read().unwrap_or_else(|e| e.into_inner());
                    if let Some(doc) = lock.document_store.get(&task.uri) {
                        (Some(doc.text.clone()), Some(doc.version))
                    } else {
                        (None, None)
                    }
                };

                // Skip if the document was closed or a newer version is already in the store.
                if let (Some(c), Some(v)) = (content, current_version) {
                    if v > version {
                        evaluator.log("Skipping parse: newer version already in store");
                        continue;
                    }

                    let parse_results = evaluator.parse_str(&c, Some(&task.uri));
                    if let Ok(results) = parse_results {
                        evaluator.log(&format!("Parsed {} forms", results.len()));
                        let mut lock = state.write().unwrap_or_else(|e| e.into_inner());
                        let uri_str = task.uri.clone();

                        let lsp_ranges: Vec<Range> = if let Some(doc) = lock.document_store.get(&uri_str) {
                             results.iter().map(|r| {
                                doc.line_index.range_from_span(&doc.text, r.line, r.col, r.span)
                            }).collect()
                        } else {
                            results.iter().map(|r| {
                                Range::new(
                                    Position::new(r.line.saturating_sub(1), r.col),
                                    Position::new(r.end_line.saturating_sub(1), r.end_col),
                                )
                            }).collect()
                        };

                        lock.ranges.insert(uri_str, lsp_ranges);

                        // Ask the client to refresh code lenses
                        let refresh_req = Request::new(
                            RequestId::from(998),
                            "workspace/codeLens/refresh".to_string(),
                            json!(null),
                        );
                        let _ = sender.send(Message::Request(refresh_req));
                        evaluator.log("Sent codeLens/refresh");
                    } else if let Err(e) = parse_results {
                        evaluator.log(&format!("Parse error: {}", e));
                    }
                }
            }
            EvalAction::Clear => {
                evaluator.log(&format!("EvalAction::Clear for {}", task.uri));
                let _ = evaluator.clear_namespace(&task.uri);
                let mut lock = state.write().unwrap_or_else(|e| e.into_inner());
                lock.results.remove(&task.uri);

                evaluator.log("Namespace cleared, sending refreshes");
                // Trigger refreshes
                let refresh_req = Request::new(
                    RequestId::from(1000),
                    "workspace/inlayHint/refresh".to_string(),
                    json!(null),
                );
                let _ = sender.send(Message::Request(refresh_req));
                let lens_refresh = Request::new(
                    RequestId::from(1001),
                    "workspace/codeLens/refresh".to_string(),
                    json!(null),
                );
                let _ = sender.send(Message::Request(lens_refresh));
            }
            EvalAction::Restart => {
                let _ = evaluator.restart();
                // Clear all stored results and ranges since they might be invalid now
                let mut lock = state.write().unwrap_or_else(|e| e.into_inner());
                lock.results.clear();
                lock.ranges.clear();
                
                // Trigger refreshes
                let refresh_req = Request::new(
                    RequestId::from(1000),
                    "workspace/inlayHint/refresh".to_string(),
                    json!(null),
                );
                let _ = sender.send(Message::Request(refresh_req));
                let lens_refresh = Request::new(
                    RequestId::from(1001),
                    "workspace/codeLens/refresh".to_string(),
                    json!(null),
                );
                let _ = sender.send(Message::Request(lens_refresh));
            }
        }
    }
}

impl Server {
    pub fn main_loop(&mut self, connection: &lsp_server::Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
        for msg in &connection.receiver {
            match msg {
                Message::Request(req) => {
                    if connection.handle_shutdown(&req)? {
                        return Ok(());
                    }
                    self.handle_request(connection, req)?;
                }
                Message::Response(_resp) => {}
                Message::Notification(not) => {
                    self.handle_notification(not)?;
                }
            }
        }
        Ok(())
    }

    pub fn handle_request(&mut self, connection: &lsp_server::Connection, req: Request) -> Result<(), Box<dyn Error + Sync + Send>> {
        if let Some(params) = cast_request::<CodeActionRequest>(&req) {
            self.handle_code_action(connection, req.id, params)?;
        } else if let Some(params) = cast_request::<ExecuteCommand>(&req) {
            self.handle_execute_command(connection, req.id, params)?;
        } else if let Some(params) = cast_request::<InlayHintRequest>(&req) {
            self.handle_inlay_hints(connection, req.id, params)?;
        } else if let Some(params) = cast_request::<CodeLensRequest>(&req) {
            self.handle_code_lens(connection, req.id, params)?;
        }
        Ok(())
    }

    pub fn handle_notification(&mut self, not: lsp_server::Notification) -> Result<(), Box<dyn Error + Sync + Send>> {
        if let Some(params) = cast_notification::<DidOpenTextDocument>(&not) {
            let uri = params.text_document.uri.to_string();
            let version = params.text_document.version;
            self.state.write().unwrap_or_else(|e| e.into_inner()).document_store.open(params.text_document);
            let _ = self.eval_tx.send(EvalTask {
                uri,
                action: EvalAction::Parse { version },
            });
        } else if let Some(params) = cast_notification::<DidChangeTextDocument>(&not) {
            let uri = params.text_document.uri.to_string();
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            
            // Heuristic shift for results before we update the document text
            let state_ref = &mut *state;
            if let Some(doc) = state_ref.document_store.get(&uri) {
                if let Some(change) = params.content_changes.first() {
                    if let Some(results) = state_ref.results.get_mut(&uri) {
                        shift_results(results, &doc.text, &change.text);
                    }
                }
            }

            state.document_store.change(
                &uri,
                params.text_document.version,
                params.content_changes,
            );
            if let Some(doc) = state.document_store.get(&uri) {
                let version = doc.version;
                let _ = self.eval_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Parse { version },
                });
            }
        } else if let Some(params) = cast_notification::<DidCloseTextDocument>(&not) {
            let uri = params.text_document.uri.to_string();
            let mut state = self.state.write().unwrap_or_else(|e| e.into_inner());
            state.document_store.close(&uri);
            state.results.remove(&uri);
            
            // Dispatch cleanup to worker
            let _ = self.eval_tx.send(EvalTask {
                uri,
                action: EvalAction::Clear,
            });
        }
        Ok(())
    }


    pub fn handle_code_action(&self, connection: &lsp_server::Connection, id: RequestId, params: CodeActionParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        let cmd = Command {
            title: "Scheme Toolbox: Evaluate File".to_string(),
            command: "scheme.evaluate".to_string(),
            arguments: Some(vec![json!(uri)]),
        };
        let action = CodeActionOrCommand::Command(cmd);
        let resp = Response::new_ok(id, Some(vec![action]));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    pub fn handle_execute_command(&mut self, connection: &lsp_server::Connection, id: RequestId, params: lsp_types::ExecuteCommandParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        if params.command == "scheme.restartREPL" {
            let _ = self.eval_tx.send(EvalTask {
                uri: "".to_string(),
                action: EvalAction::Restart,
            });
            let resp = Response::new_ok(id, json!(null));
            connection.sender.send(Message::Response(resp))?;
            return Ok(());
        }

        if params.command == "scheme.clearNamespace" {
            let uri_str = params.arguments.get(0).and_then(|v| v.as_str()).ok_or("Missing URI argument")?.to_string();
            let _ = self.eval_tx.send(EvalTask {
                uri: uri_str,
                action: EvalAction::Clear,
            });
            let resp = Response::new_ok(id, json!(null));
            connection.sender.send(Message::Response(resp))?;
            return Ok(());
        }

        let uri_str = match (params.command.as_str(), params.arguments.first().and_then(|a| a.as_str())) {
            ("scheme.evaluate" | "scheme.evaluateSelection", Some(u)) => u,
            _ => {
                let resp = Response::new_ok(id, json!(null));
                connection.sender.send(Message::Response(resp))?;
                return Ok(());
            }
        };

        let uri_str = uri_str.to_string();
        let uri = Url::parse(&uri_str)?;

        // Snapshot the content and version to evaluate at dispatch time.
        let (content_snapshot, version_snapshot, offset, byte_range) = if params.command == "scheme.evaluateSelection" {
            let content = params.arguments.get(1)
                .and_then(|a| a.as_str())
                .map(|s| s.to_string());
            
            let mut offset = None;
            let mut byte_range = None;

            if let Some(arg2) = params.arguments.get(2) {
                if let Ok(range) = serde_json::from_value::<Range>(arg2.clone()) {
                    // It's a full Range object (LSP)
                    let state = self.state.read().unwrap_or_else(|e| e.into_inner());
                    if let Some(doc) = state.document_store.get(&uri_str) {
                         let start_byte = doc.line_index.lsp_position_to_byte(&doc.text, range.start);
                         let end_byte = doc.line_index.lsp_position_to_byte(&doc.text, range.end);
                         offset = Some((range.start.line, range.start.character));
                         byte_range = Some((start_byte as u32, end_byte as u32));
                    }
                } else if let Some(o) = arg2.as_object() {
                    // Legacy {line, character} object
                    let line = o.get("line").and_then(|v| v.as_u64()).map(|v| v as u32);
                    let character = o.get("character").and_then(|v| v.as_u64()).map(|v| v as u32);
                    if let (Some(l), Some(c)) = (line, character) {
                         offset = Some((l, c));
                         let state = self.state.read().unwrap_or_else(|e| e.into_inner());
                         if let Some(doc) = state.document_store.get(&uri_str) {
                              let start_byte = doc.line_index.lsp_position_to_byte(&doc.text, Position::new(l, c));
                              // We don't have an end offset for legacy objects, so just use start for both, 
                              // or just start_byte since only start_byte_off is used for res.pos shifting anyway
                              byte_range = Some((start_byte as u32, start_byte as u32));
                         }
                    }
                }
            }
            (content, None, offset, byte_range)
        } else {
            let state = self.state.read().unwrap_or_else(|e| e.into_inner());
            let doc = state.document_store.get(&uri_str);
            let content = doc.map(|d| d.text.clone())
                .or_else(|| uri.to_file_path().ok()
                    .and_then(|p| std::fs::read_to_string(p).ok()));
            (content, doc.map(|d| d.version), None, None)
        };

        match content_snapshot {
            Some(content) => {
                // Dispatch to worker. Returns immediately.
                let _ = self.eval_tx.send(EvalTask {
                    uri: uri_str,
                    action: EvalAction::Evaluate {
                        content,
                        request_id: id.clone(),
                        version: version_snapshot,
                        offset,
                        byte_range,
                    },
                });

                // Acknowledge the request immediately.
                let resp = Response::new_ok(id, json!(null));
                connection.sender.send(Message::Response(resp))?;
            }
            None => {
                let resp = Response::new_err(
                    id,
                    lsp_server::ErrorCode::InvalidParams as i32,
                    "Could not find file or buffer content".to_string(),
                );
                connection.sender.send(Message::Response(resp))?;
            }
        }
        Ok(())
    }

    pub fn handle_inlay_hints(&self, connection: &lsp_server::Connection, id: RequestId, params: InlayHintParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let hints = if let Some(results) = state.results.get(&uri) {
            let doc = state.document_store.get(&uri);
            let doc_text = doc.map(|d| d.text.as_str());
            let line_index = doc.map(|d| &d.line_index);
            let log_handle = doc.and_then(|d| d.session_file.as_ref());
            inlay_hints::results_to_hints(results, line_index, doc_text, log_handle)
        } else {
            Vec::new()
        };
        let resp = Response::new_ok(id, Some(hints));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    pub fn handle_code_lens(&self, connection: &lsp_server::Connection, id: RequestId, params: CodeLensParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri_str = params.text_document.uri.to_string();
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        let mut lenses = Vec::new();

        if let (Some(doc), Some(ranges)) = (state.document_store.get(&uri_str), state.ranges.get(&uri_str)) {
            for range in ranges {
                let start_idx = doc.line_index.lsp_position_to_byte(&doc.text, range.start);
                let end_idx = doc.line_index.lsp_position_to_byte(&doc.text, range.end);
                // Clamp to text length
                let end_idx = end_idx.min(doc.text.len());

                let selected_text = if start_idx < end_idx {
                    &doc.text[start_idx..end_idx]
                } else {
                    ""
                };

                let cmd = Command {
                    title: "▶ Evaluate".to_string(),
                    command: "scheme.evaluateSelection".to_string(),
                    arguments: Some(vec![json!(uri_str), json!(selected_text), json!(*range)]),
                };

                lenses.push(CodeLens {
                    range: *range,
                    command: Some(cmd),
                    data: None,
                });
            }
        }

        let resp = Response::new_ok(id, Some(lenses));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }
}

pub fn cast_request<R>(req: &Request) -> Option<R::Params>
where
    R: lsp_types::request::Request,
    R::Params: serde::de::DeserializeOwned,
{
    (req.method == R::METHOD).then(|| serde_json::from_value(req.params.clone()).ok()).flatten()
}

pub fn cast_notification<N>(not: &lsp_server::Notification) -> Option<N::Params>
where
    N: lsp_types::notification::Notification,
    N::Params: serde::de::DeserializeOwned,
{
    (not.method == N::METHOD).then(|| serde_json::from_value(not.params.clone()).ok()).flatten()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};
    use std::thread;

    #[test]
    fn test_sender_non_blocking() {
        let (tx, rx) = crossbeam_channel::unbounded::<i32>();
        
        // Sending multiple messages to an unbounded channel should never block
        tx.send(1).unwrap();
        tx.send(2).unwrap();
        tx.send(3).unwrap();
        
        assert_eq!(rx.recv().unwrap(), 1);
        assert_eq!(rx.recv().unwrap(), 2);
        assert_eq!(rx.recv().unwrap(), 3);
    }

    #[test]
    fn test_lock_poisoning_recovery() {
        let state = Arc::new(RwLock::new(0));
        let state_clone = Arc::clone(&state);
        
        let _ = thread::spawn(move || {
            let mut lock = state_clone.write().unwrap();
            *lock = 1;
            panic!("Intentional panic to poison the lock");
        }).join();

        assert!(state.is_poisoned());

        // This should not panic after our fix
        let mut lock = state.write().unwrap_or_else(|poisoned| poisoned.into_inner());
        *lock = 2;
        drop(lock);

        let read_lock = state.read().unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(*read_lock, 2);
    }

    #[test]
    fn test_shift_results() {
        use crate::evaluator::EvalResult;

        let old_text = "(define x 1)\n(define y 2)\n(define z 3)";
        let new_text = ";; Comment\n(define x 1)\n(define y 2)\n(define z 3)";
        
        // Result is on (define y 2), originally line 2
        let mut results = vec![EvalResult {
            line: 2,
            col: 10,
            end_line: 2,
            end_col: 12,
            span: 2,
            pos: 14,
            result: "2".to_string(),
            is_error: false,
            output: "".to_string(),
        }];

        super::shift_results(&mut results, old_text, new_text);

        // Should be shifted to line 3
        assert_eq!(results[0].line, 3);
        assert_eq!(results[0].end_line, 3);
        // pos should be shifted by length of ";; Comment\n" (11 bytes)
        assert_eq!(results[0].pos, 14 + 11);
    }

    #[test]
    fn test_shift_results_robust() {
        use crate::evaluator::EvalResult;
        
        // Scenario: AA -> AAA
        let old_text = "AA";
        let new_text = "AAA";
        // Result at end of AA (pos 3, line 1, col 2)
        let mut results = vec![EvalResult {
            line: 1, col: 2, end_line: 1, end_col: 2, span: 0,
            pos: 3, result: "res".to_string(), is_error: false, output: "".to_string()
        }];
        
        super::shift_results(&mut results, old_text, new_text);
        
        // Common prefix "AA" (len 2). Pivot 2.
        // pos_idx = 3-1 = 2. 2 >= 2. Shifted!
        // New pos = 3 + 1 = 4.
        assert_eq!(results[0].pos, 4);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].col, 3);
        
        // Scenario: Insertion at start
        let old_text = "BB";
        let new_text = "ABB";
        let mut results = vec![EvalResult {
            line: 1, col: 1, end_line: 1, end_col: 1, span: 0,
            pos: 2, result: "res".to_string(), is_error: false, output: "".to_string()
        }];
        super::shift_results(&mut results, old_text, new_text);
        // Prefix 0. Pivot 0. 1 >= 0. Shifted.
        assert_eq!(results[0].pos, 3);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].col, 2);
    }

    #[test]
    fn test_shift_results_inside_expr() {
        use crate::evaluator::EvalResult;

        let old_text = "(define x 10)";
        let new_text = "(define x 100)";
        // Result is for the whole expression.
        // pos=1 (1-indexed), span=13 chars
        let mut results = vec![EvalResult {
            line: 1, col: 0, end_line: 1, end_col: 13, span: 13,
            pos: 1, result: "10".to_string(), is_error: false, output: "".to_string()
        }];

        super::shift_results(&mut results, old_text, new_text);

        // Edit happened at index 11 (after '10', inserting '0').
        // Pos remains 1, but span should increase by 1 to 14.
        assert_eq!(results[0].pos, 1);
        assert_eq!(results[0].span, 14);
        assert_eq!(results[0].end_col, 14);
    }

    #[test]
    fn test_merge_results() {
        use crate::evaluator::EvalResult;

        fn make_res(pos: u32, val: &str) -> EvalResult {
            EvalResult {
                line: 1, col: 0, end_line: 1, end_col: 0, span: 0,
                pos, result: val.to_string(), is_error: false, output: "".to_string()
            }
        }

        let mut existing = vec![
            make_res(10, "old1"),
            make_res(20, "old2"),
            make_res(30, "old3"),
        ];

        let new_results = vec![make_res(20, "new2")];
        
        // Merge with range covering old2 [15, 25]
        // old2 pos=20. 20-1=19. 19 is in [15, 25].
        super::merge_results(&mut existing, new_results, Some((15, 25)));

        assert_eq!(existing.len(), 3);
        assert_eq!(existing[0].result, "old1");
        assert_eq!(existing[1].result, "old3");
        assert_eq!(existing[2].result, "new2");

        // Edge case: pos=10. 10-1=9. Range [10, 20]. 
        // 9 < 10. Should NOT be cleared.
        let mut existing = vec![make_res(10, "old")];
        super::merge_results(&mut existing, vec![], Some((10, 20)));
        assert_eq!(existing.len(), 1, "Boundary case: pos=10 (idx 9) should not be cleared by range [10, 20]");

        // Edge case: pos=11. 11-1=10. Range [10, 20].
        // 10 >= 10. Should BE cleared.
        let mut existing = vec![make_res(11, "old")];
        super::merge_results(&mut existing, vec![], Some((10, 20)));
        assert_eq!(existing.len(), 0, "Boundary case: pos=11 (idx 10) should be cleared by range [10, 20]");
        
        // Full overwrite
        super::merge_results(&mut existing, vec![make_res(50, "final")], None);
        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].result, "final");
    }
}

fn merge_results(existing: &mut Vec<EvalResult>, new_results: Vec<EvalResult>, byte_range: Option<(u32, u32)>) {
    if let Some((start, end)) = byte_range {
        // Partial evaluation. Remove existing results that fall within the new range.
        // res.pos from Racket is 1-indexed, range (start/end) is 0-indexed.
        existing.retain(|res| {
            let zero_indexed_pos = res.pos.saturating_sub(1);
            zero_indexed_pos < start || zero_indexed_pos >= end
        });
        // Append new results.
        existing.extend(new_results);
    } else {
        // Full file evaluation, overwrite everything.
        *existing = new_results;
    }
}

fn normalize_results(results: &mut [EvalResult], text: &str, line_index: &LineIndex) {
    for res in results.iter_mut() {
        // Convert Racket's 1-indexed line and 0-indexed character col to a byte offset.
        let start_line_idx = res.line.saturating_sub(1) as usize;
        let start_col_idx = res.col as usize;
        
        // This correctly handles Racket's CRLF codepoint counting.
        let pos_byte_idx = line_index.byte_offset(text, start_line_idx, start_col_idx, crate::coordinates::OffsetUnit::CodePoint);
        
        // Store the byte-based position so merge_results and shift_results can use it.
        res.pos = (pos_byte_idx + 1) as u32;
    }
    recalculate_from_byte_pos(results, text, line_index);
}

fn recalculate_from_byte_pos(results: &mut [EvalResult], text: &str, line_index: &LineIndex) {
    for res in results.iter_mut() {
        let pos_byte_idx = res.pos.saturating_sub(1) as usize;
        
        // Calculate end offset by walking `span` (Unicode code points) from pos_byte_idx.
        let mut end_byte_idx = pos_byte_idx;
        let mut chars = crate::coordinates::RacketCharIndices::new(&text[pos_byte_idx.min(text.len())..]);
        for _ in 0..res.span {
            if let Some((_, s)) = chars.next() {
                end_byte_idx += s.len();
            } else {
                break;
            }
        }
        
        // Convert byte offsets back to LSP Positions (UTF-16).
        let start_pos = line_index.offset_to_position(text, pos_byte_idx);
        let end_pos = line_index.offset_to_position(text, end_byte_idx);
        
        // Update all coordinate fields to standardized UTF-16.
        res.line = start_pos.line + 1;
        res.col = start_pos.character;
        res.end_line = end_pos.line + 1;
        res.end_col = end_pos.character;
    }
}

fn shift_results(results: &mut Vec<EvalResult>, old_text: &str, new_text: &str) {
    // TODO: Refactor to optimize - potential performance impact due to string cloning and indexing on each keystroke.
    if results.is_empty() { return; }

    // let byte_delta = (new_text.len() as i32) - (old_text.len() as i32);
    // if byte_delta == 0 && old_text == new_text { return; }
    if old_text == new_text { return; }

    // Find the earliest point of divergence (common prefix)
    let mut pivot = old_text.as_bytes().iter()
        .zip(new_text.as_bytes().iter())
        .take_while(|(a, b)| a == b)
        .count();

    while pivot > 0 && (!old_text.is_char_boundary(pivot) || !new_text.is_char_boundary(pivot)) {
        pivot -= 1;
    }

    let mut common_suffix_len = old_text.as_bytes().iter().rev()
        .zip(new_text.as_bytes().iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    common_suffix_len = common_suffix_len
        .min(old_text.len() - pivot)
        .min(new_text.len() - pivot);

    while common_suffix_len > 0 {
        let old_suffix_start = old_text.len() - common_suffix_len;
        let new_suffix_start = new_text.len() - common_suffix_len;
        if old_text.is_char_boundary(old_suffix_start) && new_text.is_char_boundary(new_suffix_start) {
            break;
        }
        common_suffix_len -= 1;
    }

    let replaced_text = &old_text[pivot..old_text.len() - common_suffix_len];
    let inserted_text = &new_text[pivot..new_text.len() - common_suffix_len];

    let count_racket_chars = |s: &str| -> usize {
        crate::coordinates::RacketCharIndices::new(s).count()
    };

    let char_delta = (count_racket_chars(inserted_text) as i32) - (count_racket_chars(replaced_text) as i32);

    let new_idx = crate::coordinates::LineIndex::new(new_text);

    let byte_delta = (new_text.len() as i32) - (old_text.len() as i32);
    for res in results.iter_mut() {
        let pos_idx = res.pos.saturating_sub(1) as usize;

        let mut old_end_byte_idx = pos_idx;
        let mut chars = crate::coordinates::RacketCharIndices::new(&old_text[pos_idx.min(old_text.len())..]);
        for _ in 0..res.span {
            if let Some((_, s)) = chars.next() {
                old_end_byte_idx += s.len();
            } else {
                break;
            }
        }

        if pivot <= pos_idx {
            // Edit is before the expression
            res.pos = (res.pos as i32 + byte_delta).max(1) as u32;
        } else if pivot < old_end_byte_idx {
            // Edit is inside the expression
            res.span = (res.span as i32 + char_delta).max(1) as u32;
        }
    }

    // Standardize all coordinates to UTF-16 based on the fresh byte positions.
    recalculate_from_byte_pos(results, new_text, &new_idx);
}


