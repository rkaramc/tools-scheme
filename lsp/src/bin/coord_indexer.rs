use std::env;
use std::fs;
use std::io::Write;

use scheme_toolbox_lsp::LineIndex;

fn main() -> anyhow::Result<()> {
    // 1. Get file name from command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: coord-indexer <filename>");
        std::process::exit(1);
    }
    let input_path = &args[1];

    // 2. Get file contents
    let text = fs::read_to_string(input_path)?;

    // 3. Create line indexer
    let index = LineIndex::new(&text);

    // 4. Output a file with the requested format
    let output_path = format!("{}.coords", input_path);
    let mut output_file = fs::File::create(&output_path)?;

    let line_offsets = index.line_offsets();
    let text_len = text.len();

    for (i, &start_offset) in line_offsets.iter().enumerate() {
        // Determine the end of the line content
        let end_offset = if i + 1 < line_offsets.len() {
            line_offsets[i + 1]
        } else {
            text_len
        };

        if start_offset > text_len {
            continue;
        }

        let line_content = &text[start_offset..end_offset];
        
        // [lineno, colno, position] <line content>
        // Use LSP standard: 0-indexed for line and col (col is always 0 at line start).
        writeln!(
            output_file,
            "[{}, {}, {}] {}",
            i, 0, start_offset, line_content.escape_debug()
        )?;
    }

    println!("Output written to {}", output_path);
    Ok(())
}
