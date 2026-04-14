use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader, Write};
use anyhow::{Result, anyhow};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub line: u32,
    pub col: u32,
    pub result: String,
    pub is_error: bool,
    #[serde(default)]
    pub output: String,
}

pub struct Evaluator {
    shim_path: PathBuf,
}

impl Evaluator {
    pub fn new(shim_path: PathBuf) -> Self {
        Self { shim_path }
    }

    pub fn evaluate(&self, target_path: &PathBuf) -> Result<Vec<EvalResult>> {
        let output = Command::new("racket")
            .arg(&self.shim_path)
            .arg(target_path)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Racket evaluation failed: {}", stderr));
        }

        self.parse_output(&output.stdout)
    }

    pub fn evaluate_str(&self, content: &str) -> Result<Vec<EvalResult>> {
        let mut child = Command::new("racket")
            .arg(&self.shim_path)
            .arg("-") // Tell the shim to read from stdin
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to open stdin"))?;
        stdin.write_all(content.as_bytes())?;
        drop(stdin);

        let output = child.wait_with_output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("Racket evaluation failed: {}", stderr));
        }

        self.parse_output(&output.stdout)
    }

    fn parse_output(&self, stdout: &[u8]) -> Result<Vec<EvalResult>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_output_success() {
        let evaluator = Evaluator::new(PathBuf::from("fake-shim"));
        let json = r#"{"line":1,"col":10,"result":"42","is_error":false,"output":""}"#;
        let results = evaluator.parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].result, "42");
        assert!(!results[0].is_error);
    }

    #[test]
    fn test_parse_output_error() {
        let evaluator = Evaluator::new(PathBuf::from("fake-shim"));
        let json = r#"{"line":5,"col":5,"result":"division by zero","is_error":true,"output":""}"#;
        let results = evaluator.parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(results[0].result, "division by zero");
    }

    #[test]
    fn test_parse_output_multiple() {
        let evaluator = Evaluator::new(PathBuf::from("fake-shim"));
        let json = "{\"line\":1,\"col\":5,\"result\":\"1\",\"is_error\":false,\"output\":\"\"}\n{\"line\":2,\"col\":5,\"result\":\"2\",\"is_error\":false,\"output\":\"\"}";
        let results = evaluator.parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].result, "1");
        assert_eq!(results[1].result, "2");
    }

    #[test]
    fn test_parse_output_with_stdout() {
        let evaluator = Evaluator::new(PathBuf::from("fake-shim"));
        let json = r#"{"line":1,"col":10,"result":"void","is_error":false,"output":"hello\nworld"}"#;
        let results = evaluator.parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].output, "hello\nworld");
    }
}
