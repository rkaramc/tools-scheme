use std::str::FromStr;
use lsp_server::{Message, Request, RequestId, Response, Notification};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, DidCloseTextDocument,
    },
    request::{
        CodeActionRequest, ExecuteCommand, InlayHintRequest, CodeLensRequest,
    },
    CodeLens, Command, InlayHint, CodeActionKind, Range, Diagnostic,
};
use serde::{Deserialize, Serialize};
use std::error::Error;

use crate::documents::DocumentStore;
use crate::worker::{EvalTask, EvalAction, DiagnosticWorkerSender};
use crate::dispatch::{RequestDispatcher, NotificationDispatcher};
use crate::evaluator::EvalResult;

pub enum WorkerResult {
    EvaluateComplete {
        uri: String,
        version: Option<i32>,
        results: Vec<EvalResult>,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCellParams {
    pub uri: String,
    pub notebook_uri: Option<String>,
    pub code: String,
    pub execution_id: u32,
    pub version: Option<i32>,
}

pub enum EvalCellNotification {}
impl lsp_types::notification::Notification for EvalCellNotification {
    type Params = EvalCellParams;
    const METHOD: &'static str = "scheme/notebook/evalCell";
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelEvalParams {
    pub uri: String,
    pub execution_id: u32,
}

pub enum CancelEvalNotification {}
impl lsp_types::notification::Notification for CancelEvalNotification {
    type Params = CancelEvalParams;
    const METHOD: &'static str = "scheme/notebook/cancelEval";
}

#[derive(Debug, Deserialize)]
#[serde(tag = "command", content = "arguments")]
pub enum SchemeCommand {
    #[serde(rename = "scheme.evaluate")]
    Evaluate((String,)),
    #[serde(rename = "scheme.evaluateSelection")]
    EvaluateSelection((String, String, Range)),
    #[serde(rename = "scheme.clearNamespace")]
    ClearNamespace((String,)),
    #[serde(rename = "scheme.restartREPL")]
    #[allow(dead_code)]
    RestartREPL(Vec<serde_json::Value>),
}

pub struct Server {
    pub eval_tx: crossbeam_channel::Sender<EvalTask>,
    pub analysis_tx: crossbeam_channel::Sender<EvalTask>,
    pub cancel_tx: crossbeam_channel::Sender<u32>,
    pub document_store: DocumentStore,
    pub sender: DiagnosticWorkerSender,
}

pub enum LoopAction {
    Exit,
    Continue,
}

impl Server {
    pub fn main_loop(
        &mut self, 
        connection: &lsp_server::Connection,
        worker_rx: &crossbeam_channel::Receiver<WorkerResult>,
    ) -> Result<LoopAction, Box<dyn Error + Sync + Send>> {
        let mut shutting_down = false;
        loop {
            crossbeam_channel::select! {
                recv(&connection.receiver) -> msg => {
                    match msg {
                        Ok(Message::Request(req)) => {
                            if shutting_down {
                                continue;
                            }
                            if connection.handle_shutdown(&req)? {
                                shutting_down = true;
                                continue;
                            }
                            self.handle_request(connection, req)?;
                        }
                        Ok(Message::Response(_resp)) => {}
                        Ok(Message::Notification(not)) => {
                            if not.method == "exit" {
                                return Ok(LoopAction::Exit);
                            }
                            self.handle_notification(not)?;
                        }
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

    fn handle_worker_result(&mut self, result: WorkerResult) {
        use crate::worker::MessageSender;
        use lsp_types::{DiagnosticSeverity, Position};

        match result {
            WorkerResult::EvaluateComplete { uri, version, results, byte_range } => {
                if let Some(doc) = self.document_store.get_mut(&uri) {
                    if doc.version <= version.unwrap_or(0) {
                        merge_results(&mut doc.results, results, byte_range);
                        
                        // Build COMPOSITE diagnostics from the full results list
                        let composite_diagnostics: Vec<Diagnostic> = doc.results
                            .iter()
                            .filter(|r| r.is_error)
                            .map(|res| {
                                let range = Range::new(
                                    Position::new(res.line.saturating_sub(1), res.col),
                                    Position::new(res.end_line.saturating_sub(1), res.end_col),
                                );
                                
                                let mut severity = DiagnosticSeverity::ERROR;
                                if uri.starts_with("vscode-notebook-cell:") {
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

                        if let Ok(lsp_uri) = lsp_types::Uri::from_str(&uri) {
                            self.sender.send_diagnostics(lsp_uri, composite_diagnostics, version);
                        }
                        self.sender.refresh_inlay_hints();
                    }
                } else {
                    // Fallback for one-off evaluations (no document in store)
                    let diagnostics: Vec<Diagnostic> = results
                        .iter()
                        .filter(|r| r.is_error)
                        .map(|res| {
                            let range = Range::new(
                                Position::new(res.line.saturating_sub(1), res.col),
                                Position::new(res.end_line.saturating_sub(1), res.end_col),
                            );
                            
                            let mut severity = DiagnosticSeverity::ERROR;
                            if uri.starts_with("vscode-notebook-cell:") {
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

                    if let Ok(lsp_uri) = lsp_types::Uri::from_str(&uri) {
                        self.sender.send_diagnostics(lsp_uri, diagnostics, version);
                    }
                    self.sender.refresh_inlay_hints();
                }
            }
            WorkerResult::ParseComplete { uri, version, ranges } => {
                // eprintln!("Gateway: Received ParseComplete for {} version {}", uri, version);
                if let Some(doc) = self.document_store.get_mut(&uri) {
                    if doc.version <= version {
                        doc.ranges = ranges;
                        self.sender.refresh_code_lenses();
                    }
                }
            }
            WorkerResult::ClearNamespace { uri } => {
                // eprintln!("Gateway: Received ClearNamespace for {}", uri);
                if let Some(doc) = self.document_store.get_mut(&uri) {
                    doc.results.clear();
                    if let Ok(lsp_uri) = lsp_types::Uri::from_str(&uri) {
                        self.sender.send_diagnostics(lsp_uri, Vec::new(), None);
                    }
                    self.sender.refresh_inlay_hints();
                    self.sender.refresh_code_lenses();
                }
            }
            WorkerResult::RestartComplete => {
                eprintln!("Gateway: Received RestartComplete");
                // Clear state for ALL documents
                let uris: Vec<String> = self.document_store.iter().map(|(uri, _)| uri.clone()).collect();
                for uri in uris {
                    if let Some(doc) = self.document_store.get_mut(&uri) {
                        doc.results.clear();
                        doc.ranges.clear();
                        if let Ok(lsp_uri) = lsp_types::Uri::from_str(&uri) {
                            self.sender.send_diagnostics(lsp_uri, Vec::new(), None);
                        }
                    }
                }
                self.sender.refresh_inlay_hints();
                self.sender.refresh_code_lenses();
            }
            WorkerResult::CellEvaluationComplete { uri, version, diagnostics } => {
                // eprintln!("Gateway: Received CellEvaluationComplete for {} version {:?}", uri, version);
                if let Some(doc) = self.document_store.get_mut(&uri) {
                    if doc.version <= version.unwrap_or(0) {
                        if let Ok(lsp_uri) = lsp_types::Uri::from_str(&uri) {
                            self.sender.send_diagnostics(lsp_uri, diagnostics, version);
                        }
                    }
                }
            }
            WorkerResult::EvaluationError { uri, version, diagnostics } => {
                eprintln!("Gateway: Received EvaluationError for {} version {:?}", uri, version);
                if let Ok(lsp_uri) = lsp_types::Uri::from_str(&uri) {
                    self.sender.send_diagnostics(lsp_uri, diagnostics, version);
                }
            }
        }
    }


    pub fn handle_request(&mut self, connection: &lsp_server::Connection, req: Request) -> Result<(), Box<dyn Error + Sync + Send>> {
        let _req = RequestDispatcher::new(req)
            .on_sync_mut::<CodeActionRequest>(|id, params| self.handle_code_action(connection, id, params))?
            .on_sync_mut::<ExecuteCommand>(|id, params| self.handle_execute_command(connection, id, params))?
            .on_sync_mut::<InlayHintRequest>(|id, params| self.handle_inlay_hint(connection, id, params))?
            .on_sync_mut::<CodeLensRequest>(|id, params| self.handle_code_lens(connection, id, params))?
            .finish();
        Ok(())
    }

    pub fn handle_notification(&mut self, not: Notification) -> Result<(), Box<dyn Error + Sync + Send>> {
        let _not = NotificationDispatcher::new(not)
            .on_sync_mut::<DidOpenTextDocument>(|params| {
                let uri = params.text_document.uri.to_string();
                let version = params.text_document.version;
                eprintln!("Gateway: Opening {} (version {})", uri, version);
                self.document_store.open(params.text_document);
                let snapshot = self.document_store.get(&uri).map(|d| d.snapshot(uri.clone()));
                
                // Trigger background parse immediately on open to populate ranges for CodeLens
                if let Err(e) = self.analysis_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Parse { snapshot, version },
                }) {
                    eprintln!("analysis_tx channel full, dropping initial parse task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<DidChangeTextDocument>(|params| {
                let uri = params.text_document.uri.to_string();
                let version = params.text_document.version;
                eprintln!("Gateway: Changing {} (version {})", uri, version);
                let snapshot = if let Some(change) = params.content_changes.into_iter().last() {
                    let new_text = change.text;
                    let new_idx = crate::coordinates::LineIndex::new(&new_text);
                    self.document_store.update_text_and_index(&uri, version, new_text, new_idx);
                    self.document_store.get(&uri).map(|d| d.snapshot(uri.clone()))
                } else {
                    self.document_store.get(&uri).map(|d| d.snapshot(uri.clone()))
                };
                
                // Submit background parse task
                if let Err(e) = self.analysis_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Parse { snapshot, version },
                }) {
                    eprintln!("analysis_tx channel full, dropping parse task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<DidCloseTextDocument>(|params| {
                self.document_store.close(params.text_document.uri.as_str());
                Ok(())
            })?
            .on_sync_mut::<EvalCellNotification>(|params| {
                let snapshot = self.document_store.get(&params.uri).map(|d| d.snapshot(params.uri.clone()));
                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri: params.uri,
                    action: EvalAction::EvalCell { 
                        snapshot,
                        code: params.code, 
                        execution_id: params.execution_id,
                        notebook_uri: params.notebook_uri,
                        version: params.version,
                    },
                }) {
                    eprintln!("eval_tx channel full, dropping EvalCell task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<CancelEvalNotification>(|params| {
                if let Err(e) = self.cancel_tx.send(params.execution_id) {
                    eprintln!("cancel_tx channel full, dropping CancelEval task: {}", e);
                }
                Ok(())
            })?
            .finish();
        Ok(())
    }

    fn handle_code_action(&mut self, connection: &lsp_server::Connection, id: RequestId, params: lsp_types::CodeActionParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        
        let mut actions = Vec::new();
        
        // Add "Evaluate Selection" action if range is not empty
        if params.range.start != params.range.end {
            if let Some(doc) = self.document_store.get(&uri) {
                let text = doc.line_index.get_text_range(&doc.text, params.range);
                let cmd = Command {
                    title: "Evaluate Selection".to_string(),
                    command: "scheme.evaluateSelection".to_string(),
                    arguments: Some(vec![
                        serde_json::json!(uri),
                        serde_json::json!(text),
                        serde_json::json!(params.range),
                    ]),
                };
                actions.push(lsp_types::CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
                    title: "Evaluate Selection".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    command: Some(cmd),
                    ..Default::default()
                }));
            }
        }

        // Add "Evaluate File" action
        let cmd_all = Command {
            title: "Evaluate File".to_string(),
            command: "scheme.evaluate".to_string(),
            arguments: Some(vec![serde_json::json!(uri)]),
        };
        actions.push(lsp_types::CodeActionOrCommand::CodeAction(lsp_types::CodeAction {
            title: "Evaluate File".to_string(),
            kind: Some(CodeActionKind::QUICKFIX),
            command: Some(cmd_all),
            ..Default::default()
        }));

        let resp = Response::new_ok(id, actions);
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    fn handle_execute_command(&mut self, connection: &lsp_server::Connection, id: RequestId, params: lsp_types::ExecuteCommandParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let cmd: Result<SchemeCommand, _> = serde_json::from_value(serde_json::json!(params));
        
        match cmd {
            Ok(SchemeCommand::Evaluate((uri,))) => {
                let (snapshot, content, version) = {
                    if let Some(doc) = self.document_store.get(&uri) {
                        (Some(doc.snapshot(uri.clone())), (*doc.text).clone(), Some(doc.version))
                    } else {
                        return Ok(());
                    }
                };

                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Evaluate {
                        snapshot,
                        content,
                        request_id: id.clone(),
                        version,
                        offset: None,
                        byte_range: None,
                    },
                }) {
                    eprintln!("eval_tx channel full, dropping evaluate task: {}", e);
                }
            }
            Ok(SchemeCommand::EvaluateSelection((uri, text, range))) => {
                let (snapshot, version, byte_range) = {
                    if let Some(doc) = self.document_store.get(&uri) {
                        let start_byte = self.document_store.position_to_byte(&uri, range.start);
                        let end_byte = self.document_store.position_to_byte(&uri, range.end);
                        (Some(doc.snapshot(uri.clone())), Some(doc.version), Some((start_byte, end_byte)))
                    } else {
                        (None, None, None)
                    }
                };

                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Evaluate {
                        snapshot,
                        content: text,
                        request_id: id.clone(),
                        version,
                        offset: Some((range.start.line, range.start.character)),
                        byte_range,
                    },
                }) {
                    eprintln!("eval_tx channel full, dropping evaluateSelection task: {}", e);
                }
            }
            Ok(SchemeCommand::ClearNamespace((uri,))) => {
                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Clear,
                }) {
                    eprintln!("eval_tx channel full, dropping clear task: {}", e);
                }
            }
            Ok(SchemeCommand::RestartREPL(_)) => {
                 if let Err(e) = self.eval_tx.send(EvalTask {
                    uri: "dummy".to_string(), // Restart is global
                    action: EvalAction::Restart,
                }) {
                    eprintln!("eval_tx channel full, dropping restart task: {}", e);
                }
                if let Err(e) = self.analysis_tx.send(EvalTask {
                    uri: "dummy".to_string(), // Restart is global
                    action: EvalAction::Restart,
                }) {
                    eprintln!("analysis_tx channel full, dropping restart task: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to parse command: {}", e);
            }
        }

        let resp = Response::new_ok(id, serde_json::Value::Null);
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    fn handle_inlay_hint(&mut self, connection: &lsp_server::Connection, id: RequestId, params: lsp_types::InlayHintParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        let mut hints = Vec::new();

        if let Some(doc) = self.document_store.get(&uri) {
            eprintln!("Gateway: Found document {} with {} results for inlay hints", uri, doc.results.len());
            for res in &doc.results {
                if !res.is_error && res.result != "void" {
                    hints.push(InlayHint {
                        position: lsp_types::Position::new(res.end_line.saturating_sub(1), res.end_col),
                        label: lsp_types::InlayHintLabel::String(format!(" → {}", res.result)),
                        kind: None,
                        text_edits: None,
                        tooltip: None,
                        padding_left: Some(true),
                        padding_right: None,
                        data: None,
                    });
                }
            }
        } else {
            eprintln!("Gateway: Document {} NOT FOUND for inlay hints", uri);
        }

        let resp = Response::new_ok(id, hints);
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    fn handle_code_lens(&mut self, connection: &lsp_server::Connection, id: RequestId, params: lsp_types::CodeLensParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri_str = params.text_document.uri.to_string();
        let mut lenses = Vec::new();

        if uri_str.starts_with("vscode-notebook-cell:/") {
            let resp = Response::new_ok(id, lenses);
            connection.sender.send(Message::Response(resp))?;
            return Ok(());
        }

        if let Some(doc) = self.document_store.get(&uri_str) {
            for range in &doc.ranges {
                let selected_text = doc.line_index.get_text_range(&doc.text, *range);
                let cmd = Command {
                    title: "▶ Run".to_string(),
                    command: "scheme.evaluateSelection".to_string(),
                    arguments: Some(vec![serde_json::json!(uri_str), serde_json::json!(selected_text), serde_json::json!(*range)]),
                };

                lenses.push(CodeLens {
                    range: *range,
                    command: Some(cmd),
                    data: None,
                });
            }
        }

        let resp = Response::new_ok(id, lenses);
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }
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
