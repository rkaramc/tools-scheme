use lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, Position};
use crate::evaluator::EvalResult;

pub fn results_to_hints(results: &[EvalResult], doc_text: Option<&str>) -> Vec<InlayHint> {
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

            // `res.col` is the Unicode Code Point index (from Racket)
            // LSP `Position::character` handles UTF-16 code units. We must map the offset.
            let mut utf16_col = res.col;
            if let Some(text) = doc_text {
                if res.line > 0 {
                    let line_idx = (res.line - 1) as usize;
                    if let Some(line_str) = text.lines().nth(line_idx) {
                        let code_point_idx = res.col as usize;
                        utf16_col = line_str.chars().take(code_point_idx).map(|c| c.len_utf16() as u32).sum();
                    }
                }
            }

            InlayHint {
                position: Position::new(res.line.saturating_sub(1), utf16_col),
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
                end_col: 6,
                result: "10".to_string(),
                is_error: false,
                output: "10\n".to_string(), // Output same as result (trimmed)
            }
        ];
        let hints = results_to_hints(&results, None);
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
                end_col: 6,
                result: "other".to_string(),
                is_error: false,
                output: "hello".to_string(), // Output different from result
            }
        ];
        let hints = results_to_hints(&results, None);
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
                end_col: 6,
                result: "void".to_string(),
                is_error: false,
                output: "hello world\nline 2".to_string(),
            }
        ];
        let hints = results_to_hints(&results, None);
        assert_eq!(hints.len(), 1);
        if let InlayHintLabel::String(label) = &hints[0].label {
            assert_eq!(label, " => hello world 📝"); // Shows output instead of 'void', and has extra lines
        }
    }
}
