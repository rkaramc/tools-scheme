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

mod documents;
mod evaluator;
mod inlay_hints;
mod parser;

use documents::DocumentStore;
use evaluator::{EvalResult, Evaluator};
use parser::Parser;

struct Server {
    evaluator: Evaluator,
    parser: Parser,
    results: HashMap<String, Vec<EvalResult>>,
    document_store: DocumentStore,
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

    let server_evaluator = Evaluator::new(shim_path)
        .map_err(|e| format!("Failed to initialize evaluator: {}", e))?;

    let mut server = Server {
        evaluator: server_evaluator,
        parser: Parser::new(),
        results: HashMap::new(),
        document_store: DocumentStore::new(),
    };

    server.main_loop(&connection)?;
    io_threads.join()?;

    Ok(())
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
            self.document_store.open(params.text_document);
        } else if let Some(params) = cast_notification::<DidChangeTextDocument>(&not) {
            self.document_store.change(
                &params.text_document.uri.to_string(),
                params.text_document.version,
                params.content_changes,
            );
        } else if let Some(params) = cast_notification::<DidCloseTextDocument>(&not) {
            let uri = params.text_document.uri.to_string();
            self.document_store.close(&uri);
            self.results.remove(&uri);
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
            if let Some(arg) = params.arguments.get(0) {
                if let Some(uri_str) = arg.as_str() {
                    let uri = lsp_types::Url::parse(uri_str)?;
                    
                    let eval_results = if params.command == "scheme.evaluateSelection" {
                        if let Some(text_arg) = params.arguments.get(1) {
                            if let Some(selected_text) = text_arg.as_str() {
                                self.evaluator.evaluate_str(selected_text)
                            } else {
                                Err(anyhow::anyhow!("Invalid text argument for evaluateSelection"))
                            }
                        } else {
                            Err(anyhow::anyhow!("Missing text argument for evaluateSelection"))
                        }
                    } else if let Some(doc) = self.document_store.get(uri_str) {
                        self.evaluator.evaluate_str(&doc.text)
                    } else if let Ok(path) = uri.to_file_path() {
                        self.evaluator.evaluate(&path)
                    } else {
                        Err(anyhow::anyhow!("Could not find file or buffer content"))
                    };

                    match eval_results {
                        Ok(eval_results) => {
                            self.results.insert(uri_str.to_string(), eval_results.clone());
                            
                            let mut diagnostics = Vec::new();
                            for res in &eval_results {
                                if res.is_error {
                                    diagnostics.push(Diagnostic {
                                        range: Range::new(
                                            Position::new(res.line - 1, res.col),
                                            Position::new(res.line - 1, res.end_col),
                                        ),
                                        severity: Some(DiagnosticSeverity::ERROR),
                                        message: res.result.clone(),
                                        ..Default::default()
                                    });
                                }
                            }
                            let params = PublishDiagnosticsParams {
                                uri: uri.clone(),
                                diagnostics,
                                version: None,
                            };
                            let not = lsp_server::Notification::new(
                                PublishDiagnostics::METHOD.to_string(),
                                params,
                            );
                            connection.sender.send(Message::Notification(not))?;

                            // Send a request to refresh inlay hints
                            let refresh_req = Request::new(
                                RequestId::from(999),
                                "workspace/inlayHint/refresh".to_string(),
                                json!(null),
                            );
                            connection.sender.send(Message::Request(refresh_req))?;

                            // Return the evaluation results
                            let resp = Response::new_ok(id, json!(eval_results));
                            connection.sender.send(Message::Response(resp))?;
                            return Ok(());
                        }
                        Err(e) => {
                            let resp = Response::new_err(id, lsp_server::ErrorCode::InternalError as i32, format!("Evaluation error: {}", e));
                            connection.sender.send(Message::Response(resp))?;
                            return Ok(());
                        }
                    }
                }
            }
        }
        let resp = Response::new_ok(id, json!(null));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    fn handle_inlay_hints(&self, connection: &Connection, id: RequestId, params: InlayHintParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        let mut hints = Vec::new();

        if let Some(results) = self.results.get(&uri) {
            let doc_text = self.document_store.get(&uri).map(|d| d.text.as_str());
            hints = inlay_hints::results_to_hints(results, doc_text);
        }

        let resp = Response::new_ok(id, Some(hints));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    fn handle_code_lens(&self, connection: &Connection, id: RequestId, params: CodeLensParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri_str = params.text_document.uri.to_string();
        let mut lenses = Vec::new();

        if let Some(doc) = self.document_store.get(&uri_str) {
            let ranges = self.parser.find_top_level_expressions(&doc.text);
            for range in ranges {
                // Extract the text for this range
                let start_idx = doc.text.lines().take(range.start.line as usize).map(|l| l.len() + 1).sum::<usize>() + range.start.character as usize;
                let mut end_idx = doc.text.lines().take(range.end.line as usize).map(|l| l.len() + 1).sum::<usize>() + range.end.character as usize;
                // Basic clamp to avoid out-of-bounds
                if end_idx > doc.text.len() { end_idx = doc.text.len(); }
                
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
