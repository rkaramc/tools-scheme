use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, Position};
use crate::coordinates::LineIndex;
use crate::evaluator::EvalResult;

pub fn results_to_hints(results: &[EvalResult], line_index: Option<&LineIndex>, doc_text: Option<&str>) -> Vec<InlayHint> {
    results.iter()
        .filter(|res| !res.is_error)
        .map(|res| {
            let result_trimmed = res.result.trim();
            let output_trimmed = res.output.trim();
            
            // Determine the display value
            let display_val = if (res.result == "'void" || res.result == "#<void>") && !res.output.is_empty() {
                res.output.lines().next().unwrap_or("").trim().to_string()
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
 
            // Convert Racket's code-point column to LSP's UTF-16 column at the END of the expression.
            let position = match (line_index, doc_text) {
                (Some(idx), Some(text)) => {
                    let range = idx.range_from_span(text, res.line, res.col, res.span);
                    range.end
                }
                _ => {
                    let lsp_end_line = if res.end_line > 0 { res.end_line.saturating_sub(1) } else { res.line.saturating_sub(1) };
                    Position::new(lsp_end_line, res.end_col)
                }
            };
 
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
            }
        ];
        let hints = results_to_hints(&results, None, None);
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
            }
        ];
        let hints = results_to_hints(&results, None, None);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, "  => other 📝"); // Includes notebook icon
        }
        assert!(hints[0].tooltip.is_some());
    }

    #[test]
    fn test_results_to_hints_void_with_output() {
        let results = vec![
            EvalResult {
                line: 1,
                col: 5,
                end_line: 1,
                end_col: 6,
                span: 1,
                pos: 6,
                result: "'void".to_string(),
                is_error: false,
                output: "hello world\nline 2".to_string(),
            }
        ];
        let hints = results_to_hints(&results, None, None);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, "  => hello world 📝"); // Shows output instead of 'void', and has extra lines
        }
    }
}
