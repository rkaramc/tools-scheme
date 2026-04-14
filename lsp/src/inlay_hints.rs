use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, Position};
use crate::evaluator::EvalResult;

pub fn results_to_hints(results: &[EvalResult]) -> Vec<InlayHint> {
    results.iter()
        .filter(|res| !res.is_error)
        .map(|res| {
            let label = if res.output.is_empty() {
                format!(" => {}", res.result)
            } else {
                format!(" => {} 📝", res.result)
            };
            
            let tooltip = if res.output.is_empty() {
                None
            } else {
                Some(InlayHintTooltip::String(res.output.clone()))
            };

            InlayHint {
                position: Position::new(res.line - 1, res.col),
                label: InlayHintLabel::String(label),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip,
                padding_left: Some(true),
                padding_right: None,
                data: None,
            }
        })
        .collect()
}
