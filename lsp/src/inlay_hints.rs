use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, Position};
use crate::evaluator::EvalResult;

pub fn results_to_hints(results: &[EvalResult]) -> Vec<InlayHint> {
    results.iter()
        .filter(|res| !res.is_error)
        .map(|res| {
            let result_trimmed = res.result.trim();
            let output_trimmed = res.output.trim();
            
            // Determine the display value
            let display_val = if (res.result == "void" || res.result == "#<void>") && !res.output.is_empty() {
                res.output.lines().next().unwrap_or("void").trim().to_string()
            } else {
                res.result.clone()
            };

            // If result and output are the same, show only result
            let has_extra_output = !res.output.is_empty() && result_trimmed != output_trimmed;

            let label = if has_extra_output {
                format!(" => {} 📝", display_val)
            } else {
                format!(" => {}", display_val)
            };
            
            let tooltip = if has_extra_output {
                Some(InlayHintTooltip::String(res.output.clone()))
            } else {
                None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_results_to_hints_simplified() {
        let results = vec![
            EvalResult {
                line: 1,
                col: 5,
                result: "10".to_string(),
                is_error: false,
                output: "10\n".to_string(), // Output same as result (trimmed)
            }
        ];
        let hints = results_to_hints(&results);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, " => 10"); // No notebook icon
        }
        assert!(hints[0].tooltip.is_none());
    }

    #[test]
    fn test_results_to_hints_extra_output() {
        let results = vec![
            EvalResult {
                line: 1,
                col: 5,
                result: "other".to_string(),
                is_error: false,
                output: "hello".to_string(), // Output different from result
            }
        ];
        let hints = results_to_hints(&results);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, " => other 📝"); // Includes notebook icon
        }
        assert!(hints[0].tooltip.is_some());
    }

    #[test]
    fn test_results_to_hints_void_with_output() {
        let results = vec![
            EvalResult {
                line: 1,
                col: 5,
                result: "void".to_string(),
                is_error: false,
                output: "hello world\nline 2".to_string(),
            }
        ];
        let hints = results_to_hints(&results);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, " => hello world 📝"); // Shows output instead of 'void', and has extra lines
        }
    }
}
