use lsp_server::{Connection, Message, Request, RequestId, Response};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification,
        PublishDiagnostics,
    },
    request::{CodeActionRequest, CodeLensRequest, ExecuteCommand, InlayHintRequest},
    CodeActionKind, CodeActionOptions, CodeActionOrCommand, CodeActionParams, CodeLens,
    CodeLensOptions, CodeLensParams, Command, Diagnostic, DiagnosticSeverity, InlayHintParams,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkDoneProgressOptions,
};
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::sync::mpsc;

mod coordinates;
mod documents;
mod evaluator;
mod inlay_hints;
mod parser;

use documents::DocumentStore;
use evaluator::{EvalResult, Evaluator};
use parser::Parser;

/// State shared between the main loop and the eval worker thread.
struct SharedState {
    results: HashMap<String, Vec<EvalResult>>,
    document_store: DocumentStore,
}

/// A request to evaluate Scheme content, dispatched to the worker thread.
struct EvalTask {
    uri: String,
    /// Snapshot of document content taken at dispatch time.
    content: String,
    request_id: RequestId,
}

struct Server {
    eval_tx: mpsc::SyncSender<EvalTask>,
    state: Arc<RwLock<SharedState>>,
    parser: Parser,
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    // Parse arguments to find the shim path
    let args: Vec<String> = std::env::args().collect();
    let shim_path = if let Some(path_arg) = args.get(1) {
        PathBuf::from(path_arg)
    } else {
        // Fallback 1: check environment variable
        let env_fallback = std::env::var("TOOLS_SCHEME_LSP_PATH")
            .map(|s| PathBuf::from(s).join("eval-shim.rkt"))
            .ok()
            .filter(|p| p.exists());

        if let Some(p) = env_fallback {
            p
        } else {
            // Fallback 2: look for eval-shim.rkt in the same directory as the executable
            let mut path = std::env::current_exe()?;
            path.pop();
            path.push("eval-shim.rkt");
            if !path.exists() {
                // Third fallback: dev path
                std::env::current_dir()?.join("lsp/src/eval-shim.rkt")
            } else {
                path
            }
        }
    };

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
            work_done_progress_options: WorkDoneProgressOptions::default(),
            resolve_provider: Some(false),
        })),
        inlay_hint_provider: Some(lsp_types::OneOf::Left(true)),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        execute_command_provider: Some(lsp_types::ExecuteCommandOptions {
            commands: vec![
                "scheme.evaluate".to_string(),
                "scheme.evaluateSelection".to_string(),
            ],
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let _initialization_params = connection.initialize(server_capabilities)?;

    let evaluator = Evaluator::new(shim_path)
        .map_err(|e| format!("Failed to initialize evaluator: {}", e))?;

    let state = Arc::new(RwLock::new(SharedState {
        results: HashMap::new(),
        document_store: DocumentStore::new(),
    }));

    // Channel capacity of 1: the worker processes one task at a time. A second
    // send while one is in-flight will block the main loop briefly until the
    // slot is free — acceptable, since evaluation is user-initiated.
    let (eval_tx, eval_rx) = mpsc::sync_channel::<EvalTask>(1);

    // Spawn the eval worker. It owns the Evaluator (and thus the Racket REPL
    // child process) and is the only thread that ever calls into it.
    let worker_state = Arc::clone(&state);
    let worker_sender = connection.sender.clone();
    std::thread::spawn(move || {
        eval_worker(evaluator, eval_rx, worker_state, worker_sender);
    });

    let mut server = Server {
        eval_tx,
        state,
        parser: Parser::new(),
    };

    server.main_loop(&connection)?;
    io_threads.join()?;

    Ok(())
}

/// Background thread: receives EvalTask, evaluates, updates SharedState, sends notifications.
fn eval_worker(
    mut evaluator: Evaluator,
    rx: mpsc::Receiver<EvalTask>,
    state: Arc<RwLock<SharedState>>,
    sender: crossbeam_channel::Sender<Message>,
) {
    for task in rx {
        let eval_results = evaluator.evaluate_str(&task.content);

        let uri_str = task.uri.clone();
        let uri = match lsp_types::Url::parse(&uri_str) {
            Ok(u) => u,
            Err(_) => continue,
        };

        match eval_results {
            Ok(results) => {
                // Build diagnostics while we still have the results in hand,
                // before acquiring the write lock.
                let diagnostics: Vec<Diagnostic> = {
                    let state_read = state.read().unwrap();
                    let doc = state_read.document_store.get(&uri_str);
                    results
                        .iter()
                        .filter(|r| r.is_error)
                        .map(|res| {
                            let lsp_line = res.line.saturating_sub(1);
                            let (start_col, end_col) = match doc {
                                Some(d) => (
                                    d.line_index.code_point_to_utf16(&d.text, lsp_line as usize, res.col as usize),
                                    d.line_index.code_point_to_utf16(&d.text, lsp_line as usize, res.end_col as usize),
                                ),
                                None => (res.col, res.end_col),
                            };
                            Diagnostic {
                                range: Range::new(
                                    Position::new(lsp_line, start_col),
                                    Position::new(lsp_line, end_col),
                                ),
                                severity: Some(DiagnosticSeverity::ERROR),
                                message: res.result.clone(),
                                ..Default::default()
                            }
                        })
                        .collect()
                };

                // Store results.
                state.write().unwrap().results.insert(uri_str.clone(), results);

                // Publish diagnostics.
                let diag_params = PublishDiagnosticsParams {
                    uri: uri.clone(),
                    diagnostics,
                    version: None,
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
                    version: None,
                };
                let not = lsp_server::Notification::new(
                    PublishDiagnostics::METHOD.to_string(),
                    diag_params,
                );
                let _ = sender.send(Message::Notification(not));
            }
        }
    }
}

impl Server {
    fn main_loop(&mut self, connection: &Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
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

    fn handle_request(&mut self, connection: &Connection, req: Request) -> Result<(), Box<dyn Error + Sync + Send>> {
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

    fn handle_notification(&mut self, not: lsp_server::Notification) -> Result<(), Box<dyn Error + Sync + Send>> {
        if let Some(params) = cast_notification::<DidOpenTextDocument>(&not) {
            self.state.write().unwrap().document_store.open(params.text_document);
        } else if let Some(params) = cast_notification::<DidChangeTextDocument>(&not) {
            self.state.write().unwrap().document_store.change(
                params.text_document.uri.as_ref(),
                params.text_document.version,
                params.content_changes,
            );
        } else if let Some(params) = cast_notification::<DidCloseTextDocument>(&not) {
            let uri = params.text_document.uri.to_string();
            let mut state = self.state.write().unwrap();
            state.document_store.close(&uri);
            state.results.remove(&uri);
        }
        Ok(())
    }

    fn handle_code_action(&self, connection: &Connection, id: RequestId, params: CodeActionParams) -> Result<(), Box<dyn Error + Sync + Send>> {
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

    fn handle_execute_command(&mut self, connection: &Connection, id: RequestId, params: lsp_types::ExecuteCommandParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        if params.command == "scheme.evaluate" || params.command == "scheme.evaluateSelection" {
            if let Some(arg) = params.arguments.first() {
                if let Some(uri_str) = arg.as_str() {
                    let uri_str = uri_str.to_string();
                    let uri = lsp_types::Url::parse(&uri_str)?;

                    // Snapshot the content to evaluate at dispatch time.
                    let content_snapshot = if params.command == "scheme.evaluateSelection" {
                        params.arguments.get(1)
                            .and_then(|a| a.as_str())
                            .map(|s| s.to_string())
                    } else {
                        let state = self.state.read().unwrap();
                        state.document_store.get(&uri_str)
                            .map(|d| d.text.clone())
                            .or_else(|| uri.to_file_path().ok()
                                .and_then(|p| std::fs::read_to_string(p).ok()))
                    };

                    match content_snapshot {
                        Some(content) => {
                            // Dispatch to worker. Returns immediately.
                            let _ = self.eval_tx.send(EvalTask {
                                uri: uri_str,
                                content,
                                request_id: id.clone(),
                            });
                            // Acknowledge the request immediately. Results arrive
                            // via PublishDiagnostics and inlayHint/refresh notifications.
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
                    return Ok(());
                }
            }
        }
        let resp = Response::new_ok(id, json!(null));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    fn handle_inlay_hints(&self, connection: &Connection, id: RequestId, params: InlayHintParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        let state = self.state.read().unwrap();
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

    fn handle_code_lens(&self, connection: &Connection, id: RequestId, params: CodeLensParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri_str = params.text_document.uri.to_string();
        let state = self.state.read().unwrap();
        let mut lenses = Vec::new();

        if let Some(doc) = state.document_store.get(&uri_str) {
            let ranges = self.parser.find_top_level_expressions(&doc.text);
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
                    range,
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

fn cast_request<R>(req: &Request) -> Option<R::Params>
where
    R: lsp_types::request::Request,
    R::Params: serde::de::DeserializeOwned,
{
    if req.method == R::METHOD {
        serde_json::from_value(req.params.clone()).ok()
    } else {
        None
    }
}

fn cast_notification<N>(not: &lsp_server::Notification) -> Option<N::Params>
where
    N: lsp_types::notification::Notification,
    N::Params: serde::de::DeserializeOwned,
{
    if not.method == N::METHOD {
        serde_json::from_value(not.params.clone()).ok()
    } else {
        None
    }
}
