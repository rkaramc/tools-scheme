use std::sync::{Arc, RwLock};
use std::sync::atomic::AtomicI32;
use std::error::Error;
use std::str::FromStr;
use url::Url;
use lsp_server::{Message, Request, RequestId};
use lsp_types::{
    notification::{Notification as _, PublishDiagnostics},
    Diagnostic, DiagnosticSeverity, Position, PublishDiagnosticsParams, Range,
};
use crate::server::{SharedState, SharedStateExt};
use crate::evaluator::{EvalResult, Evaluator};
use crate::coordinates::LineIndex;

static NEXT_REQ_ID: AtomicI32 = AtomicI32::new(100);

pub trait MessageSender {
    fn send_diagnostics(&self, uri: lsp_types::Uri, diagnostics: Vec<Diagnostic>, version: Option<i32>);
    fn refresh_inlay_hints(&self);
    fn refresh_code_lenses(&self);
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
}

pub enum EvalAction {
    Evaluate { 
        content: String, 
        request_id: RequestId, 
        version: Option<i32>, 
        offset: Option<(u32, u32)>, 
        byte_range: Option<(u32, u32)> 
    },
    Parse { version: i32 },
    Clear,
    Restart,
}

pub struct EvalTask {
    pub uri: String,
    pub action: EvalAction,
}

pub fn eval_worker(
    mut evaluator: Evaluator,
    rx: crossbeam_channel::Receiver<EvalTask>,
    state: Arc<RwLock<SharedState>>,
    sender: crossbeam_channel::Sender<Message>,
) {
    for task in rx {
        match task.action {
            EvalAction::Evaluate { content, version, offset, byte_range, request_id } => {
                on_evaluate(&mut evaluator, &state, &sender, &task.uri, content, version, offset, byte_range, request_id);
            }
            EvalAction::Parse { version } => {
                on_parse(&mut evaluator, &state, &sender, &task.uri, version);
            }
            EvalAction::Clear => {
                on_clear(&mut evaluator, &state, &sender, &task.uri);
            }
            EvalAction::Restart => {
                on_restart(&mut evaluator, &state, &sender);
            }
        }
    }
}

fn on_evaluate(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &crossbeam_channel::Sender<Message>,
    uri_str: &str,
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

    let log_handle = state.read_recovered()
        .document_store.get(uri_str)
        .and_then(|d| d.session_file.as_ref())
        .and_then(|f| f.try_clone().ok());

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
                    res.pos += start_byte_off as u32;
                }
            }

            // Normalize coordinates to UTF-16 immediately using syntax-position and span.
            if let Some(doc) = state.read_recovered().document_store.get(uri_str) {
                if is_selection {
                    recalculate_from_byte_pos(&mut results, &doc.text, &doc.line_index);
                } else {
                    normalize_results(&mut results, &doc.text, &doc.line_index);
                }
            }

            // Build diagnostics while we still have the results in hand
            let diagnostics: Vec<Diagnostic> = results
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
                .collect();

            // Store results with spatial merging.
            {
                let mut lock = state.write_recovered();
                if let Some(doc) = lock.document_store.get_mut(uri_str) {
                    merge_results(&mut doc.results, results, byte_range);
                }
            }

            // Publish diagnostics.
            sender.send_diagnostics(uri, diagnostics, version);

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
    sender: &crossbeam_channel::Sender<Message>,
    uri_str: &str,
    version: i32,
) {
    evaluator.log(&format!("EvalAction::Parse(version: {:?}) for {}", version, uri_str));
    let (content, current_version) = {
        let lock = state.read_recovered();
        if let Some(doc) = lock.document_store.get(uri_str) {
            (Some(doc.text.clone()), Some(doc.version))
        } else {
            (None, None)
        }
    };

    if let (Some(c), Some(v)) = (content, current_version) {
        if v > version {
            evaluator.log("Skipping parse: newer version already in store");
            return;
        }

        let parse_results = evaluator.parse_str(&c, Some(uri_str));
        if let Ok(results) = parse_results {
            evaluator.log(&format!("Parsed {} forms", results.len()));
            let mut lock = state.write_recovered();

            if let Some(doc) = lock.document_store.get_mut(uri_str) {
                 let lsp_ranges: Vec<Range> = results.iter().map(|r| {
                    doc.line_index.range_from_span(&doc.text, r.line, r.col, r.span)
                }).collect();
                doc.ranges = lsp_ranges;
            }

            // Ask the client to refresh code lenses
            sender.refresh_code_lenses();
        } else if let Err(e) = parse_results {
            evaluator.log(&format!("Parse error: {}", e));
        }
    }
}

fn on_clear(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &crossbeam_channel::Sender<Message>,
    uri_str: &str,
) {
    evaluator.log(&format!("EvalAction::Clear for {}", uri_str));
    let _ = evaluator.clear_namespace(uri_str);
    let mut lock = state.write_recovered();
    if let Some(doc) = lock.document_store.get_mut(uri_str) {
        doc.results.clear();
    }

    evaluator.log("Namespace cleared, sending refreshes");
    
    // Trigger refreshes
    sender.refresh_inlay_hints();
    sender.refresh_code_lenses();
}

fn on_restart(
    evaluator: &mut Evaluator,
    state: &Arc<RwLock<SharedState>>,
    sender: &crossbeam_channel::Sender<Message>,
) {
    let _ = evaluator.restart();
    let mut lock = state.write_recovered();
    for doc in lock.document_store.iter_mut() {
        doc.results.clear();
        doc.ranges.clear();
    }
    
    // Trigger refreshes
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

    fn make_res(pos: u32, val: &str) -> EvalResult {
        EvalResult {
            line: 1, col: 0, end_line: 1, end_col: 0, span: 0,
            pos, result: val.to_string(), is_error: false, output: "".to_string()
        }
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
