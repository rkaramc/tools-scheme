use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio, Child, ChildStdin, ChildStdout};
use std::io::{BufRead, BufReader, Write};
use anyhow::{Result, anyhow};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub line: u32,
    pub col: u32,
    #[serde(default)]
    pub end_col: u32,
    pub result: String,
    pub is_error: bool,
    #[serde(default)]
    pub output: String,
}

pub struct Evaluator {
    stdin: ChildStdin,
    stdout_reader: BufReader<ChildStdout>,
    _child: Child,
    session_file: std::fs::File,
}

impl Evaluator {
    pub fn new(shim_path: PathBuf) -> Result<Self> {
        let session_file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(".session")?;

        let mut child = Command::new("racket")
            .arg(shim_path)
            .arg("--repl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(session_file.try_clone()?))
            .spawn()?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to open stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to open stdout"))?;
        let stdout_reader = BufReader::new(stdout);

        Ok(Self {
            stdin,
            stdout_reader,
            _child: child,
            session_file,
        })
    }

    pub fn evaluate(&mut self, target_path: &PathBuf) -> Result<Vec<EvalResult>> {
        let content = std::fs::read_to_string(target_path)?;
        self.evaluate_str(&content)
    }

    pub fn evaluate_str(&mut self, content: &str) -> Result<Vec<EvalResult>> {
        writeln!(&mut self.session_file, "\n--- EVAL INPUT ---\n{}\n--- EVAL OUTPUT ---", content)?;
        self.session_file.flush()?;

        let req = serde_json::json!({
            "type": "evaluate",
            "content": content
        });
        
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;

        let mut results = Vec::new();
        let mut buffer = String::new();
        
        loop {
            buffer.clear();
            let n = self.stdout_reader.read_line(&mut buffer)?;
            if n == 0 {
                return Err(anyhow!("REPL process exited unexpectedly"));
            }

            self.session_file.write_all(buffer.as_bytes())?;
            self.session_file.flush()?;
            
            let trimmed = buffer.trim();
            if trimmed == "READY" {
                break;
            }
            
            if let Ok(res) = serde_json::from_str::<EvalResult>(trimmed) {
                results.push(res);
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to simulate the parsing logic for tests
    fn parse_output(stdout: &[u8]) -> Result<Vec<EvalResult>> {
        let mut results = Vec::new();
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line?;
            if let Ok(res) = serde_json::from_str::<EvalResult>(&line) {
                results.push(res);
            }
        }
        Ok(results)
    }

    #[test]
    fn test_parse_json_result() {
        let json = r#"{"line":1,"col":10,"result":"42","is_error":false,"output":""}"#;
        let res: EvalResult = serde_json::from_str(json).unwrap();
        assert_eq!(res.line, 1);
        assert_eq!(res.result, "42");
        assert!(!res.is_error);
    }

    #[test]
    fn test_parse_output_success() {
        let json = r#"{"line":1,"col":10,"result":"42","is_error":false,"output":""}"#;
        let results = parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].result, "42");
        assert!(!results[0].is_error);
    }

    #[test]
    fn test_parse_output_error() {
        let json = r#"{"line":5,"col":5,"result":"division by zero","is_error":true,"output":""}"#;
        let results = parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(results[0].result, "division by zero");
    }

    #[test]
    fn test_parse_output_multiple() {
        let json = "{\"line\":1,\"col\":5,\"result\":\"1\",\"is_error\":false,\"output\":\"\"}\n{\"line\":2,\"col\":5,\"result\":\"2\",\"is_error\":false,\"output\":\"\"}";
        let results = parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].result, "1");
        assert_eq!(results[1].result, "2");
    }

    #[test]
    fn test_parse_output_with_stdout() {
        let json = r#"{"line":1,"col":10,"result":"void","is_error":false,"output":"hello\nworld"}"#;
        let results = parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].output, "hello\nworld");
    }
}
