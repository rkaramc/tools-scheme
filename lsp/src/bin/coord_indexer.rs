use std::env;
use std::fs;
use std::io::Write;

#[path = "../coordinates.rs"]
mod coordinates;

use coordinates::LineIndex;

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

        // If it's the very last offset and it's equal to text_len, 
        // it might represent an empty line after a trailing newline.
        // We skip it if the previous line already covered up to EOF,
        // unless we want to represent that empty line.
        // Standard behavior: text[start..end] gives the line content.
        
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

// I need to add a getter for line_offsets to LineIndex if it's not pub.
// Wait, I should check coordinates.rs again.
