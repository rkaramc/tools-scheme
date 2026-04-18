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

/// State shared between the main loop and the eval worker thread.
pub struct SharedState {
    pub results: HashMap<String, Vec<EvalResult>>,
    pub ranges: HashMap<String, Vec<Range>>,
    pub document_store: DocumentStore,
}

pub enum EvalAction {
    Evaluate { content: String, request_id: RequestId, version: Option<i32> },
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
            EvalAction::Evaluate { content, version, .. } => {
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

                let eval_results = evaluator.evaluate_str(&content, Some(&uri_str), context_label.as_deref(), log_handle.as_ref());

                match eval_results {
                    Ok(results) => {
                        // Build diagnostics while we still have the results in hand,
                        // before acquiring the write lock.
                        let diagnostics: Vec<Diagnostic> = {
                            let state_read = state.read().unwrap_or_else(|e| e.into_inner());
                            let doc = state_read.document_store.get(&uri_str);
                            results
                                .iter()
                                .filter(|r| r.is_error)
                                        .map(|res| {
                                            let range = match doc {
                                                Some(d) => d.line_index.range_from_span(&d.text, res.line, res.col, res.span),
                                                None => {
                                                    // Fallback if doc is not in store
                                                    let lsp_start_line = res.line.saturating_sub(1);
                                                    let lsp_end_line = if res.end_line > 0 { res.end_line.saturating_sub(1) } else { lsp_start_line };
                                                    Range::new(
                                                        Position::new(lsp_start_line, res.col),
                                                        Position::new(lsp_end_line, res.end_col),
                                                    )
                                                }
                                            };
                                            Diagnostic {
                                                range,
                                                severity: Some(DiagnosticSeverity::ERROR),
                                                message: res.result.clone(),
                                                ..Default::default()
                                            }
                                        })
                                .collect()
                        };

                        // Store results.
                        state.write().unwrap_or_else(|e| e.into_inner()).results.insert(uri_str.clone(), results);

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
                        continue;
                    }

                    let parse_results = evaluator.parse_str(&c, Some(&task.uri));
                    if let Ok(results) = parse_results {
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
                    }
                }
            }
            EvalAction::Clear => {
                let _ = evaluator.clear_namespace(&task.uri);
                let mut lock = state.write().unwrap_or_else(|e| e.into_inner());
                lock.ranges.remove(&task.uri);
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
        let (content_snapshot, version_snapshot) = if params.command == "scheme.evaluateSelection" {
            let content = params.arguments.get(1)
                .and_then(|a| a.as_str())
                .map(|s| s.to_string());
            (content, None)
        } else {
            let state = self.state.read().unwrap_or_else(|e| e.into_inner());
            let doc = state.document_store.get(&uri_str);
            let content = doc.map(|d| d.text.clone())
                .or_else(|| uri.to_file_path().ok()
                    .and_then(|p| std::fs::read_to_string(p).ok()));
            (content, doc.map(|d| d.version))
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
            inlay_hints::results_to_hints(results, line_index, doc_text)
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
                    arguments: Some(vec![json!(uri_str), json!(selected_text)]),
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
}
