use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use lsp_server::{Message, Request, RequestId, Response, Notification};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidOpenTextDocument, DidCloseTextDocument,
    },
    request::{
        CodeActionRequest, ExecuteCommand, InlayHintRequest, CodeLensRequest,
    },
    CodeLens, Command, InlayHint, CodeActionKind, Range,
};
use serde::{Deserialize, Serialize};
use std::error::Error;

use crate::documents::DocumentStore;
use crate::worker::{EvalTask, EvalAction};
use crate::dispatch::{RequestDispatcher, NotificationDispatcher};

pub struct SharedState {
    pub document_store: DocumentStore,
}

pub trait RwLockExt<T> {
    fn read_recovered(&self) -> RwLockReadGuard<'_, T>;
    fn write_recovered(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_recovered(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| e.into_inner())
    }
    fn write_recovered(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| e.into_inner())
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
    pub state: Arc<RwLock<SharedState>>,
}

pub enum LoopAction {
    Exit,
    Continue,
}

impl Server {
    pub fn read_state(&'_ self) -> RwLockReadGuard<'_, SharedState> {
        self.state.read_recovered()
    }

    pub fn write_state(&'_ self) -> RwLockWriteGuard<'_, SharedState> {
        self.state.write_recovered()
    }

    pub fn main_loop(&mut self, connection: &lsp_server::Connection) -> Result<LoopAction, Box<dyn Error + Sync + Send>> {
        let mut shutting_down = false;
        for msg in &connection.receiver {
            match msg {
                Message::Request(req) => {
                    if shutting_down {
                        continue;
                    }
                    if connection.handle_shutdown(&req)? {
                        shutting_down = true;
                        continue;
                    }
                    self.handle_request(connection, req)?;
                }
                Message::Response(_resp) => {}
                Message::Notification(not) => {
                    if not.method == "exit" {
                        return Ok(LoopAction::Exit);
                    }
                    self.handle_notification(not)?;
                }
            }
        }
        Ok(LoopAction::Continue)
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
                self.write_state().document_store.open(params.text_document);
                
                // Trigger background parse immediately on open to populate ranges for CodeLens
                if let Err(e) = self.analysis_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Parse { version },
                }) {
                    eprintln!("analysis_tx channel full, dropping initial parse task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<DidChangeTextDocument>(|params| {
                let uri = params.text_document.uri.to_string();
                let version = params.text_document.version;
                if let Some(change) = params.content_changes.into_iter().last() {
                    let mut lock = self.write_state();
                    let new_text = change.text;
                    let new_idx = crate::coordinates::LineIndex::new(&new_text);
                    lock.document_store.update_text_and_index(&uri, version, new_text, new_idx);
                }
                
                // Submit background parse task
                if let Err(e) = self.analysis_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Parse { version },
                }) {
                    eprintln!("analysis_tx channel full, dropping parse task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<DidCloseTextDocument>(|params| {
                self.write_state().document_store.close(params.text_document.uri.as_str());
                Ok(())
            })?
            .on_sync_mut::<EvalCellNotification>(|params| {
                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri: params.uri,
                    action: EvalAction::EvalCell { 
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
            if let Some(doc) = self.read_state().document_store.get(&uri) {
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
                let (content, version) = {
                    let lock = self.read_state();
                    if let Some(doc) = lock.document_store.get(&uri) {
                        (doc.text.clone(), Some(doc.version))
                    } else {
                        return Ok(());
                    }
                };

                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Evaluate {
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
                let (version, byte_range) = {
                    let lock = self.read_state();
                    if let Some(doc) = lock.document_store.get(&uri) {
                        let start_byte = lock.document_store.position_to_byte(&uri, range.start);
                        let end_byte = lock.document_store.position_to_byte(&uri, range.end);
                        (Some(doc.version), Some((start_byte, end_byte)))
                    } else {
                        (None, None)
                    }
                };

                if let Err(e) = self.eval_tx.send(EvalTask {
                    uri,
                    action: EvalAction::Evaluate {
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

        if let Some(doc) = self.read_state().document_store.get(&uri) {
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

        if let Some(doc) = self.read_state().document_store.get(&uri_str) {
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
