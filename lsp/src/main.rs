use lsp_server::{Connection, Message, Request, RequestId, Response};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification,
        PublishDiagnostics,
    },
    request::{CodeActionRequest, ExecuteCommand, InlayHintRequest},
    CodeActionKind, CodeActionOptions, CodeActionOrCommand, CodeActionParams, Command,
    Diagnostic, DiagnosticSeverity, InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams,
    Position, PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkDoneProgressOptions,
};
use serde_json::json;
use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

mod evaluator;
use evaluator::{EvalResult, Evaluator};

struct Server {
    evaluator: Evaluator,
    results: HashMap<String, Vec<EvalResult>>,
    documents: HashMap<String, String>,
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
        execute_command_provider: Some(lsp_types::ExecuteCommandOptions {
            commands: vec!["scheme.evaluate".to_string()],
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let _initialization_params = connection.initialize(server_capabilities)?;

    let mut server = Server {
        evaluator: Evaluator::new(shim_path),
        results: HashMap::new(),
        documents: HashMap::new(),
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
        }
        Ok(())
    }

    fn handle_notification(&mut self, not: lsp_server::Notification) -> Result<(), Box<dyn Error + Sync + Send>> {
        if let Some(params) = cast_notification::<DidOpenTextDocument>(&not) {
            self.documents.insert(params.text_document.uri.to_string(), params.text_document.text);
        } else if let Some(params) = cast_notification::<DidChangeTextDocument>(&not) {
            if let Some(change) = params.content_changes.into_iter().last() {
                self.documents.insert(params.text_document.uri.to_string(), change.text);
            }
        } else if let Some(params) = cast_notification::<DidCloseTextDocument>(&not) {
            let uri = params.text_document.uri.to_string();
            self.documents.remove(&uri);
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
        if params.command == "scheme.evaluate" {
            if let Some(arg) = params.arguments.get(0) {
                if let Some(uri_str) = arg.as_str() {
                    let uri = lsp_types::Url::parse(uri_str)?;
                    
                    let eval_results = if let Some(content) = self.documents.get(uri_str) {
                        self.evaluator.evaluate_str(content)
                    } else if let Ok(path) = uri.to_file_path() {
                        self.evaluator.evaluate(&path)
                    } else {
                        Err(anyhow::anyhow!("Could not find file or buffer content"))
                    };

                    if let Ok(eval_results) = eval_results {
                        self.results.insert(uri_str.to_string(), eval_results.clone());
                        
                        // Publish diagnostics for errors
                        let mut diagnostics = Vec::new();
                        for res in &eval_results {
                            if res.is_error {
                                diagnostics.push(Diagnostic {
                                    range: Range::new(
                                        Position::new(res.line - 1, res.col),
                                        Position::new(res.line - 1, 999),
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
            for res in results {
                if res.is_error {
                    continue;
                }
                let label = if res.output.is_empty() {
                    format!(" => {}", res.result)
                } else {
                    format!(" => {} 📝", res.result)
                };
                let tooltip = if res.output.is_empty() {
                    None
                } else {
                    Some(lsp_types::InlayHintTooltip::String(res.output.clone()))
                };
                let hint = InlayHint {
                    position: Position::new(res.line - 1, res.col),
                    label: InlayHintLabel::String(label),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                };
                hints.push(hint);
            }
        }

        let resp = Response::new_ok(id, Some(hints));
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
