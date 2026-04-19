use std::env;
use std::fs;

use lsp_types::Range;
use scheme_toolbox_lsp::{LineIndex, Evaluator, inlay_hints};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: coord-compare <filename>");
        std::process::exit(1);
    }
    let input_path = &args[1];
    
    // 1. Get file contents & build LineIndex
    let text = fs::read_to_string(input_path)?;
    let index = LineIndex::new(&text);

    // 2. Run eval-shim via Evaluator
    let mut ev = Evaluator::new(None)?;
    let uri = format!("file:///{}", std::fs::canonicalize(input_path)?.to_string_lossy().replace('\\', "/"));
    let context_label = std::path::Path::new(input_path).file_name().map(|n| n.to_string_lossy().into_owned());
    let results = ev.evaluate_str(&text, Some(&uri), context_label.as_deref(), None)?;

    let valid_results: Vec<_> = results.into_iter().filter(|r| !r.is_error).collect();
    let hints = inlay_hints::results_to_hints(&valid_results, Some(&index), Some(&text), None);

    println!("{:<5} | {:<5} {:<5} | {:<5} {:<5} | {:<5} | {:<16} | {:<6} {:<6} | {:<6} {:<6} | {:<15} | {:<15} | {:<15}",
             "Line", 
             "rkt_s", "s_cp",
             "rkt_e", "e_cp",
             "r_spn",
             "lsp_range",
             "Start(u16)", 
             "End(u16)", 
             "H.Line",
             "H.Col",
             "Result",
             "Label",
             "Tooltip");
    println!("{:-<5}-+-{:-<5}-{:-<5}-+-{:-<5}-{:-<5}-+-{:-<5}-+-{:-<16}-+-{:-<6}-{:-<6}-+-{:-<6}-{:-<6}-+-{:-<15}-+-{:-<15}-+-{:-<15}-",
             "", "", "", "", "", "", "", "", "", "", "", "", "", "");
    
    for (json, hint) in valid_results.into_iter().zip(hints) {
        // Racket output is 1-indexed for lines, 0-indexed for columns
        let rkt_line_0 = json.line.saturating_sub(1) as usize;
        let rkt_end_line_0 = json.end_line.saturating_sub(1) as usize;
        
        // Shim uses code points
        let start_cp = json.col as usize;
        let end_cp = json.end_col as usize;

        let rkt_span = json.span as usize;
        
        // LSP uses utf16
        let lsp_range = index.range_from_span(&text, json.line, json.col, json.span);

        let start_utf16 = index.code_point_to_utf16(&text, rkt_line_0, start_cp);
        let end_utf16 = index.code_point_to_utf16(&text, rkt_end_line_0, end_cp);
        
        let label_str = match hint.label {
            lsp_types::InlayHintLabel::String(s) => s,
            _ => String::from("..."),
        };
        
        let tooltip_str = match hint.tooltip {
            Some(lsp_types::InlayHintTooltip::String(s)) => s,
            _ => String::from("None"),
        };
        
        println!("{:<5} | {:<5} {:<5} | {:<5} {:<5} | {:<5} | {:<16} | {:<6} {:<6} | {:<6} {:<6} | {:<15} | {:<15} | {:<15}",
            json.line,
            rkt_line_0,
            start_cp,
            rkt_end_line_0,
            end_cp,
            rkt_span,
            range_to_display(lsp_range),
            // offset,
            start_utf16,
            end_utf16,
            hint.position.line,
            hint.position.character,
            json.result.replace('\n', "\\n").chars().take(15).collect::<String>(),
            label_str.replace('\n', "\\n").chars().take(15).collect::<String>(),
            tooltip_str.replace('\n', "\\n").chars().take(15).collect::<String>()
        );
    }

    Ok(())
}

fn range_to_display(range: Range) -> String {
    format!("{:>3},{:>3}->{:>3},{:>3}", range.start.line, range.start.character, range.end.line, range.end.character)
}