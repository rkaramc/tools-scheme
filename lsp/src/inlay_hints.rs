use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, Position};
use crate::coordinates::LineIndex;
use crate::evaluator::EvalResult;

pub fn results_to_hints(results: &[EvalResult], _line_index: Option<&LineIndex>, _doc_text: Option<&str>, _log: Option<&std::fs::File>) -> Vec<InlayHint> {
    results.iter()
        .filter(|res| !res.is_error)
        .map(|res| {
            let result_trimmed = res.result.trim();
            let output_trimmed = res.output.trim();
            
            // Determine the display value
            let display_val = if (res.result == "'void" || res.result == "#<void>") && !res.output.is_empty() {
                res.output.trim().replace('\n', " ↵ ")
            } else {
                res.result.clone()
            };

            // If result and output are the same, show only result
            let has_extra_output = !res.output.is_empty() && result_trimmed != output_trimmed;

            let label = if has_extra_output {
                format!("  => {} 📝", display_val)
            } else {
                format!("  => {}", display_val)
            };
            
            let tooltip = if has_extra_output {
                Some(InlayHintTooltip::String(res.output.clone()))
            } else {
                None
            };
 
            // After normalize_results, end_line/end_col are already LSP UTF-16.
            let lsp_end_line = if res.end_line > 0 { res.end_line } else { res.line };
            let position = Position::new(lsp_end_line.saturating_sub(1), res.end_col);
 
            InlayHint {
                position,
                label: InlayHintLabel::String(label),
                kind: Some(InlayHintKind::PARAMETER),
                text_edits: None,
                tooltip,
                padding_left: Some(false), // Matches suffix style better
                padding_right: None,
                data: None,
            }
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_results_to_hints_simplified() {
        let results = vec![
            EvalResult {
                line: 1,
                col: 5,
                end_line: 1,
                end_col: 6,
                span: 1,
                pos: 6,
                result: "10".to_string(),
                is_error: false,
                output: "10\n".to_string(), // Output same as result (trimmed)
                kind: "code".to_string(),
            }
        ];
        let hints = results_to_hints(&results, None, None, None);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, "  => 10"); // No notebook icon
        }
        assert!(hints[0].tooltip.is_none());
    }

    #[test]
    fn test_results_to_hints_extra_output() {
        let results = vec![
            EvalResult {
                line: 1,
                col: 5,
                end_line: 1,
                end_col: 6,
                span: 1,
                pos: 6,
                result: "other".to_string(),
                is_error: false,
                output: "hello".to_string(), // Output different from result
                kind: "code".to_string(),
            }
        ];
        let hints = results_to_hints(&results, None, None, None);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, "  => other 📝"); // Includes notebook icon
        }
        assert!(hints[0].tooltip.is_some());
    }

    #[test]
    fn test_results_to_hints_void_with_output() {
        let results = vec![EvalResult {
            line: 1,
            col: 5,
            end_line: 1,
            end_col: 10,
            span: 5,
            pos: 10,
            result: "'void".to_string(),
            is_error: false,
            output: "hello\nworld".to_string(),
            kind: "code".to_string(),
        }];
        let hints = results_to_hints(&results, None, None, None);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, "  => hello ↵ world 📝"); // Shows output collapsed, instead of 'void'
        }
    }
}
