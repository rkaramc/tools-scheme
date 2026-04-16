use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio, Child, ChildStdin};
use std::io::{BufRead, BufReader, Write};
use anyhow::{Result, anyhow};
use std::path::PathBuf;
use std::fs::File;
use std::time::Duration;
use crossbeam_channel::Receiver;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub line: u32,
    pub col: u32,
    #[serde(default)]
    pub end_line: u32,
    #[serde(default)]
    pub end_col: u32,
    pub result: String,
    pub is_error: bool,
    #[serde(default)]
    pub output: String,
}

pub struct Evaluator {
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
    child: Child,
    path: PathBuf,
    timeout: Duration,
    global_session: std::fs::File,
}

impl Evaluator {
    pub fn new(shim_path: PathBuf) -> Result<Self> {
        let global_session = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("global.session")?;

        let timeout_secs = std::env::var("TOOLS_SCHEME_EVAL_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);
        let timeout = Duration::from_secs(timeout_secs);

        let (stdin, stdout_rx, child) = Self::spawn_process(&shim_path, &global_session)?;

        Ok(Self {
            stdin,
            stdout_rx,
            child,
            path: shim_path,
            timeout,
            global_session,
        })
    }

    fn spawn_process(shim_path: &PathBuf, session_file: &File) -> Result<(ChildStdin, Receiver<String>, Child)> {
        let mut child = Command::new("racket")
            .arg(shim_path)
            .arg("--repl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(session_file.try_clone()?))
            .spawn()?;

        let stdin = child.stdin.take().ok_or_else(|| anyhow!("Failed to open stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| anyhow!("Failed to open stdout"))?;
        
        let (tx, rx) = crossbeam_channel::unbounded();
        
        // Background reader thread
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                if let Ok(l) = line {
                    if tx.send(l).is_err() {
                        break;
                    }
                } else {
                    break;
                }
            }
        });

        Ok((stdin, rx, child))
    }

    fn ensure_alive(&mut self) -> Result<()> {
        let is_dead = match self.child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => true,
        };

        if is_dead {
            let (stdin, stdout_rx, child) = Self::spawn_process(&self.path, &self.global_session)?;
            self.stdin = stdin;
            self.stdout_rx = stdout_rx;
            self.child = child;
        }
        Ok(())
    }

    pub fn evaluate(&mut self, target_path: &PathBuf) -> Result<Vec<EvalResult>> {
        let content = std::fs::read_to_string(target_path)?;
        self.evaluate_str(&content, None)
    }

    pub fn evaluate_str(&mut self, content: &str, log: Option<&File>) -> Result<Vec<EvalResult>> {
        self.ensure_alive()?;

        if let Some(mut file) = log {
            writeln!(file, "\n--- EVAL INPUT ---\n{}\n--- EVAL OUTPUT ---", content)?;
            file.flush()?;
        } else {
            writeln!(&mut self.global_session, "\n--- EVAL INPUT (NO LOG) ---\n{}\n--- EVAL OUTPUT ---", content)?;
            self.global_session.flush()?;
        }

        let req = serde_json::json!({
            "type": "evaluate",
            "content": content
        });
        
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        
        if self.stdin.write_all(line.as_bytes()).is_err() {
            // If pipe is broken, try restarting and retrying once
            self.child.kill()?;
            self.ensure_alive()?;
            self.stdin.write_all(line.as_bytes())?;
        }
        self.stdin.flush()?;

        let mut results = Vec::new();
        
        loop {
            let buffer = match self.stdout_rx.recv_timeout(self.timeout) {
                Ok(l) => l,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Err(anyhow!("Evaluation timed out after {:?}", self.timeout));
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return Err(anyhow!("REPL process exited unexpectedly"));
                }
            };

            if let Some(mut file) = log {
                writeln!(file, "{}", buffer)?;
                file.flush()?;
            } else {
                writeln!(&mut self.global_session, "{}", buffer)?;
                self.global_session.flush()?;
            }
            
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

impl Drop for Evaluator {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
        let json = r#"{"line":1,"col":10,"end_line":1,"end_col":12,"result":"42","is_error":false,"output":""}"#;
        let res: EvalResult = serde_json::from_str(json).unwrap();
        assert_eq!(res.line, 1);
        assert_eq!(res.end_line, 1);
        assert_eq!(res.result, "42");
        assert!(!res.is_error);
    }

    #[test]
    fn test_parse_output_success() {
        let json = r#"{"line":1,"col":10,"end_line":1,"end_col":12,"result":"42","is_error":false,"output":""}"#;
        let results = parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line, 1);
        assert_eq!(results[0].result, "42");
        assert!(!results[0].is_error);
    }

    #[test]
    fn test_parse_output_error() {
        let json = r#"{"line":5,"col":5,"end_line":5,"end_col":10,"result":"division by zero","is_error":true,"output":""}"#;
        let results = parse_output(json.as_bytes()).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].is_error);
        assert_eq!(results[0].result, "division by zero");
    }

    #[test]
    fn test_parse_output_multiple() {
        let json = "{\"line\":1,\"col\":5,\"end_line\":1,\"end_col\":10,\"result\":\"1\",\"is_error\":false,\"output\":\"\"}\n{\"line\":2,\"col\":5,\"end_line\":2,\"end_col\":10,\"result\":\"2\",\"is_error\":false,\"output\":\"\"}";
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

    #[test]
    fn test_evaluation_timeout() {
        // Find the actual shim path for a real test
        let mut shim_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        shim_path.push("src");
        shim_path.push("eval-shim.rkt");
        
        if !shim_path.exists() {
            // Skip if we can't find the shim (e.g. in some CI environments)
            return;
        }

        let mut evaluator = Evaluator::new(shim_path).unwrap();
        // Set a very short timeout for the test
        evaluator.timeout = Duration::from_millis(500);

        // Infinite loop: (let loop () (loop))
        let result = evaluator.evaluate_str("(let loop () (loop))", None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));

        // Verify recovery: subsequent evaluation should work (after restart)
        evaluator.timeout = Duration::from_secs(5);
        let result = evaluator.evaluate_str("42", None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].result, "42");
    }
}
