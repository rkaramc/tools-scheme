use lsp_server::{Message, Request, RequestId, Response};
use crate::dispatch::{RequestDispatcher, NotificationDispatcher};
use lsp_types::{
    notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
    },
    request::{CodeActionRequest, CodeLensRequest, ExecuteCommand, InlayHintRequest}, 
    CodeActionOrCommand, CodeActionParams, CodeLens,
    CodeLensParams, Command, InlayHintParams,
    Position, Range,
};
use serde::Deserialize;
use serde_json::json;
use url::Url;
use std::error::Error;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use crate::documents::DocumentStore;
use crate::evaluator::{EvalResult};
use crate::inlay_hints;

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalCellParams {
    pub uri: String,
    pub notebook_uri: Option<String>,
    pub code: String,
    pub execution_id: u32,
}

pub enum EvalCellNotification {}
impl lsp_types::notification::Notification for EvalCellNotification {
    type Params = EvalCellParams;
    const METHOD: &'static str = "scheme/notebook/evalCell";
}

#[derive(Debug, Eq, PartialEq, Clone, Deserialize, serde::Serialize)]
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
enum SchemeCommand {
    #[serde(rename = "scheme.evaluate")]
    Evaluate((String,)),
    #[serde(rename = "scheme.evaluateSelection")]
    EvaluateSelection((String, String, SelectionRange)),
    #[serde(rename = "scheme.clearNamespace")]
    ClearNamespace((String,)),
    #[serde(rename = "scheme.restartREPL")]
    RestartREPL,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SelectionRange {
    Modern(Range),
    Legacy { line: u32, character: u32 },
}

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

use crate::worker::{EvalAction, EvalTask};
pub struct Server {
    pub eval_tx: crossbeam_channel::Sender<EvalTask>,
    pub cancel_tx: crossbeam_channel::Sender<u32>,
    pub state: Arc<RwLock<SharedState>>,
}

impl Server {
    pub fn read_state(&'_ self) -> RwLockReadGuard<'_, SharedState> {
        self.state.read_recovered()
    }

    pub fn write_state(&'_ self) -> RwLockWriteGuard<'_, SharedState> {
        self.state.write_recovered()
    }

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
        let _req = RequestDispatcher::new(req)
            .on_sync_mut::<CodeActionRequest>(|id, params| self.handle_code_action(connection, id, params))?
            .on_sync_mut::<ExecuteCommand>(|id, params| self.handle_execute_command(connection, id, params))?
            .on_sync_mut::<InlayHintRequest>(|id, params| self.handle_inlay_hints(connection, id, params))?
            .on_sync_mut::<CodeLensRequest>(|id, params| self.handle_code_lens(connection, id, params))?
            .finish();
        Ok(())
    }

    pub fn handle_notification(&mut self, not: lsp_server::Notification) -> Result<(), Box<dyn Error + Sync + Send>> {
        let _not = NotificationDispatcher::new(not)
            .on_sync_mut::<DidOpenTextDocument>(|params| {
                let uri = params.text_document.uri.to_string();
                let version = params.text_document.version;
                self.write_state().document_store.open(params.text_document);
                if let Err(e) = self.eval_tx.try_send(EvalTask {
                    uri,
                    action: EvalAction::Parse { version },
                }) {
                    eprintln!("eval_tx channel full, dropping parse task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<DidChangeTextDocument>(|params| {
                let uri = params.text_document.uri.to_string();
                let mut state = self.write_state();
                
                if let Some(change) = params.content_changes.into_iter().last() {
                    let new_text = change.text;
                    let new_idx = crate::coordinates::LineIndex::new(&new_text);
                    
                    let state_ref = &mut *state;
                    if let Some(doc) = state_ref.document_store.get_mut(&uri) {
                        shift_results(&mut doc.results, &doc.text, &new_text, &new_idx);
                        doc.version = params.text_document.version;
                        doc.text = new_text;
                        doc.line_index = new_idx;

                        let _ = self.eval_tx.send(EvalTask {
                            uri,
                            action: EvalAction::Parse { version: doc.version },
                        });
                    }
                }
                Ok(())
            })?
            .on_sync_mut::<DidCloseTextDocument>(|params| {
                let uri = params.text_document.uri.to_string();
                let mut state = self.write_state();
                state.document_store.close(&uri);
                
                // Dispatch cleanup to worker
                if let Err(e) = self.eval_tx.try_send(EvalTask {
                    uri,
                    action: EvalAction::Clear,
                }) {
                    eprintln!("eval_tx channel full, dropping Clear task: {}", e);
                }
                Ok(())
            })?
            .on_sync_mut::<EvalCellNotification>(|params| {
                let task = EvalTask {
                    uri: params.uri,
                    action: EvalAction::EvalCell {
                        code: params.code,
                        execution_id: params.execution_id,
                        notebook_uri: params.notebook_uri,
                    },
                };
                if let Err(e) = self.eval_tx.try_send(task) {
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
        let cmd: SchemeCommand = match serde_json::from_value(json!(params)) {
            Ok(c) => c,
            Err(_) => {
                // Not one of our commands, acknowledge and return
                connection.sender.send(Message::Response(Response::new_ok(id, json!(null))))?;
                return Ok(());
            }
        };

        let msg_response = match cmd {
            SchemeCommand::RestartREPL => {
                self.send_eval_task(id, EvalTask { uri: "".into(), action: EvalAction::Restart })
            }
            SchemeCommand::ClearNamespace((uri,)) => {
                self.send_eval_task(id, EvalTask { uri, action: EvalAction::Clear })
            }
            SchemeCommand::Evaluate((uri_str,)) => {
                let (content, version) = self.get_document_snapshot(&uri_str)?;
                
                if let Some(content) = content {
                    self.send_eval_task(id.clone(), EvalTask {
                        uri: uri_str,
                        action: EvalAction::Evaluate {
                            content,
                            request_id: id,
                            version,
                            offset: None,
                            byte_range: None,
                        },
                    })
                } else {
                    Message::Response(Response::new_err(
                        id,
                        lsp_server::ErrorCode::InvalidParams as i32,
                        "Could not find file or buffer content".into(),
                    ))
                }
            }
            SchemeCommand::EvaluateSelection((uri_str, content, sel)) => {
                let (offset, byte_range) = self.get_selection_offsets(&uri_str, &sel);

                self.send_eval_task(id.clone(), EvalTask {
                    uri: uri_str,
                    action: EvalAction::Evaluate {
                        content,
                        request_id: id,
                        version: None,
                        offset,
                        byte_range,
                    },
                })
            }
        };

        // Acknowledge all commands
        connection.sender.send(msg_response)?;
        Ok(())
    }

    fn send_eval_task(&self, id: RequestId, task: EvalTask) -> Message {
        if self.eval_tx.send(task).is_err() {
            Message::Response(Response::new_err(
                id,
                lsp_server::ErrorCode::InternalError as i32,
                "Evaluation worker disconnected".into(),
            ))
        } else {
            Message::Response(Response::new_ok(id, json!(null)))
        }
    }

    fn get_document_snapshot(&self, uri_str: &str) -> Result<(Option<String>, Option<i32>), Box<dyn Error + Sync + Send>> {
        let state = self.read_state();
        let doc = state.document_store.get(uri_str);
        
        let content = doc.map(|d| d.text.clone())
            .or_else(|| {
                Url::parse(uri_str).ok()
                    .and_then(|u| u.to_file_path().ok())
                    .and_then(|p| std::fs::read_to_string(p).ok())
            });
        let version = doc.map(|d| d.version);
        
        Ok((content, version))
    }
}

type SelectionOffsets = (Option<(u32, u32)>, Option<(u32, u32)>);

impl Server {
    fn get_selection_offsets(&self, uri_str: &str, sel: &SelectionRange) -> SelectionOffsets {
        let mut offset = None;
        let mut byte_range = None;

        match sel {
            SelectionRange::Modern(range) => {
                let state = self.read_state();
                if let Some(doc) = state.document_store.get(uri_str) {
                     let start_byte = doc.line_index.lsp_position_to_byte(&doc.text, range.start);
                     let end_byte = doc.line_index.lsp_position_to_byte(&doc.text, range.end);
                     offset = Some((range.start.line, range.start.character));
                     byte_range = Some((start_byte as u32, end_byte as u32));
                }
            }
            SelectionRange::Legacy { line, character } => {
                offset = Some((*line, *character));
                let state = self.read_state();
                if let Some(doc) = state.document_store.get(uri_str) {
                     let start_byte = doc.line_index.lsp_position_to_byte(&doc.text, Position::new(*line, *character));
                     byte_range = Some((start_byte as u32, start_byte as u32));
                }
            }
        }
        (offset, byte_range)
    }

    pub fn handle_inlay_hints(&self, connection: &lsp_server::Connection, id: RequestId, params: InlayHintParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri = params.text_document.uri.to_string();
        let state = self.read_state();
        let hints = if let Some(doc) = state.document_store.get(&uri) {
            let doc_text = Some(doc.text.as_str());
            let line_index = Some(&doc.line_index);
            let log_handle = doc.session_file.as_ref();
            inlay_hints::results_to_hints(&doc.results, line_index, doc_text, log_handle)
        } else {
            Vec::new()
        };
        let resp = Response::new_ok(id, Some(hints));
        connection.sender.send(Message::Response(resp))?;
        Ok(())
    }

    pub fn handle_code_lens(&self, connection: &lsp_server::Connection, id: RequestId, params: CodeLensParams) -> Result<(), Box<dyn Error + Sync + Send>> {
        let uri_str = params.text_document.uri.to_string();
        let state = self.read_state();
        let mut lenses = Vec::new();

        if let Some(doc) = state.document_store.get(&uri_str) {
            for range in &doc.ranges {
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


#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};
    use std::thread;
    use super::RwLockExt;

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
        let mut lock = state.write_recovered();
        *lock = 2;
        drop(lock);

        let read_lock = state.read_recovered();
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

        let new_idx = crate::coordinates::LineIndex::new(new_text);
        super::shift_results(&mut results, old_text, new_text, &new_idx);

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
        
        let new_idx = crate::coordinates::LineIndex::new(new_text);
        super::shift_results(&mut results, old_text, new_text, &new_idx);
        
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
        let new_idx2 = crate::coordinates::LineIndex::new(new_text);
        super::shift_results(&mut results, old_text, new_text, &new_idx2);
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

        let new_idx = crate::coordinates::LineIndex::new(new_text);
        super::shift_results(&mut results, old_text, new_text, &new_idx);

        // Edit happened at index 11 (after '10', inserting '0').
        // Pos remains 1, but span should increase by 1 to 14.
        assert_eq!(results[0].pos, 1);
        assert_eq!(results[0].span, 14);
        assert_eq!(results[0].end_col, 14);
    }

}

// merge_results, normalize_results, recalculate_from_byte_pos moved to worker.rs

fn shift_results(results: &mut [EvalResult], old_text: &str, new_text: &str, new_idx: &crate::coordinates::LineIndex) {
    if results.is_empty() { return; }

    let byte_delta = (new_text.len() as i32) - (old_text.len() as i32);
    if byte_delta == 0 && old_text == new_text { return; }

    // Find the earliest point of divergence (common prefix)
    let mut pivot = old_text.as_bytes().iter()
        .zip(new_text.as_bytes().iter())
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| old_text.len().min(new_text.len()));

    while pivot > 0 && (!old_text.is_char_boundary(pivot) || !new_text.is_char_boundary(pivot)) {
        pivot -= 1;
    }

    let mut lazy_char_delta: Option<i32> = None;

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
            let char_delta = *lazy_char_delta.get_or_insert_with(|| {
                let mut common_suffix_len = old_text.as_bytes().iter().rev()
                    .zip(new_text.as_bytes().iter().rev())
                    .position(|(a, b)| a != b)
                    .unwrap_or_else(|| old_text.len().min(new_text.len()));

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

                (count_racket_chars(inserted_text) as i32) - (count_racket_chars(replaced_text) as i32)
            });

            res.span = (res.span as i32 + char_delta).max(1) as u32;
        }
    }

    // Standardize all coordinates to UTF-16 based on the fresh byte positions.
    crate::worker::recalculate_from_byte_pos(results, new_text, new_idx);
}


