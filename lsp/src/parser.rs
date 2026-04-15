use lsp_types::{Position, Range};

pub struct Parser;

impl Parser {
    pub fn new() -> Self {
        Self
    }
    
    pub fn find_top_level_expressions(&self, text: &str) -> Vec<Range> {
        let mut ranges = Vec::new();
        
        let mut depth = 0;
        let mut in_string = false;
        let mut start_line = 0;
        let mut start_col = 0;
        
        let mut curr_line = 0;
        let mut curr_col = 0;

        let mut iter = text.chars().peekable();
        
        while let Some(c) = iter.next() {
            match c {
                '\n' => {
                    curr_line += 1;
                    curr_col = 0;
                    continue; // Skip the column increment below
                }
                '"' => {
                    in_string = !in_string;
                }
                '\\' if in_string => {
                    // Skip escaped character
                    if let Some(&next_c) = iter.peek() {
                        if next_c == '\n' {
                            curr_line += 1;
                            curr_col = 0;
                        } else {
                            curr_col += 1;
                        }
                        iter.next();
                    }
                }
                ';' if !in_string => {
                    // Skip rest of line
                    while let Some(&next_c) = iter.peek() {
                        if next_c == '\n' {
                            break;
                        }
                        iter.next();
                        curr_col += 1;
                    }
                }
                '(' | '[' | '{' if !in_string => {
                    if depth == 0 {
                        start_line = curr_line;
                        start_col = curr_col;
                    }
                    depth += 1;
                }
                ')' | ']' | '}' if !in_string => {
                    depth -= 1;
                    if depth == 0 {
                        ranges.push(Range {
                            start: Position::new(start_line as u32, start_col as u32),
                            end: Position::new(curr_line as u32, curr_col as u32 + 1),
                        });
                    }
                }
                _ => {}
            }
            curr_col += 1;
        }
        
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_top_level_expressions() {
        let parser = Parser::new();
        let text = "(define x 1)\n\n(define (foo)\n  (+ 1 2))\n\n; A comment\n(display \"hello\")";
        let ranges = parser.find_top_level_expressions(text);
        
        assert_eq!(ranges.len(), 3);
        
        assert_eq!(ranges[0].start.line, 0);
        assert_eq!(ranges[0].start.character, 0);
        assert_eq!(ranges[0].end.line, 0);
        assert_eq!(ranges[0].end.character, 12);
        
        assert_eq!(ranges[1].start.line, 2);
        assert_eq!(ranges[1].start.character, 0);
        assert_eq!(ranges[1].end.line, 3);
        assert_eq!(ranges[1].end.character, 10);
        
        assert_eq!(ranges[2].start.line, 6);
        assert_eq!(ranges[2].start.character, 0);
        assert_eq!(ranges[2].end.line, 6);
        assert_eq!(ranges[2].end.character, 17);
    }
}


