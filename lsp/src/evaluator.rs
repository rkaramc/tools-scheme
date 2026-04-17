use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio, Child, ChildStdin};
use std::io::{BufRead, BufReader, Write};
use anyhow::{Result, anyhow};
use std::path::{Path, PathBuf};
use std::fs::File;
use std::time::Duration;
use crossbeam_channel::Receiver;
use std::sync::atomic::{AtomicUsize, Ordering};

const SHIM_SOURCE: &str = include_str!("eval-shim.rkt");
static SHIM_COUNTER: AtomicUsize = AtomicUsize::new(0);
const TEMP_SUBDIR: &str = "vscode-scheme-toolbox-lsp";

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RangeResult {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

struct ProcessState {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
}

pub struct Evaluator {
    state: Option<ProcessState>,
    shim_path: PathBuf,
    _shim_lock: Option<std::fs::File>,
    timeout: Duration,
    global_session: std::fs::File,
}

impl Evaluator {
    pub fn new() -> Result<Self> {
        let global_session = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("global.session")?;

        let timeout_secs = std::env::var("TOOLS_SCHEME_EVAL_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);
        let timeout = Duration::from_secs(timeout_secs);

        // Prepare the embedded shim in a temporary location (consolidated folder)
        let counter = SHIM_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(TEMP_SUBDIR);
        std::fs::create_dir_all(&temp_dir)?;
        let shim_path = temp_dir.join(format!("eval-shim-{}-{}.rkt", std::process::id(), counter));
        
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1 | 2); // FILE_SHARE_READ | FILE_SHARE_WRITE (No DELETE)
        }
        
        let mut shim_lock = options.open(&shim_path)?;
        shim_lock.write_all(SHIM_SOURCE.as_bytes())?;
        shim_lock.flush()?;

        let state = Self::spawn_process(&shim_path, &global_session)?;

        Ok(Self {
            state: Some(state),
            shim_path,
            _shim_lock: Some(shim_lock),
            timeout,
            global_session,
        })
    }

    fn spawn_process(shim_path: &PathBuf, session_file: &File) -> Result<ProcessState> {
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

        Ok(ProcessState {
            child,
            stdin,
            stdout_rx: rx,
        })
    }

    fn ensure_alive(&mut self) -> Result<&mut ProcessState> {
        let needs_restart = match &mut self.state {
            Some(state) => {
                match state.child.try_wait() {
                    Ok(Some(_)) => true,
                    Ok(None) => false,
                    Err(_) => true,
                }
            }
            None => true,
        };

        if needs_restart {
            // Drop old state explicitly (kills process via Drop if implemented, or we do it here)
            if let Some(mut old_state) = self.state.take() {
                let _ = old_state.child.kill();
                let _ = old_state.child.wait();
            }
            self.state = Some(Self::spawn_process(&self.shim_path, &self.global_session)?);
        }
        
        Ok(self.state.as_mut().unwrap())
    }

    pub fn evaluate(&mut self, target_path: &PathBuf) -> Result<Vec<EvalResult>> {
        let content = std::fs::read_to_string(target_path)?;
        let uri = format!("file:///{}", target_path.to_string_lossy());
        self.evaluate_str(&content, Some(&uri), None)
    }

    pub fn evaluate_str(&mut self, content: &str, uri: Option<&str>, log: Option<&File>) -> Result<Vec<EvalResult>> {

        if let Some(mut file) = log {
            writeln!(file, "\n--- EVAL INPUT ---\n{}\n--- EVAL OUTPUT ---", content)?;
            file.flush()?;
        } else {
            writeln!(&mut self.global_session, "\n--- EVAL INPUT (NO LOG) ---\n{}\n--- EVAL OUTPUT ---", content)?;
            self.global_session.flush()?;
        }

        let state = self.ensure_alive()?;

        let req = serde_json::json!({
            "type": "evaluate",
            "content": content,
            "uri": uri
        });

        
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        
        let mut retry = false;
        if state.stdin.write_all(line.as_bytes()).is_err() {
            retry = true;
        } else {
            let _ = state.stdin.flush();
        }

        if retry {
            // If pipe is broken, kill the state and retry once
            self.state.take();
            let new_state = self.ensure_alive()?;
            new_state.stdin.write_all(line.as_bytes())?;
            new_state.stdin.flush()?;
        }

        let mut results = Vec::new();
        
        loop {
            // Re-borrow state since we might have replaced it above
            let state = self.state.as_mut().unwrap();
            
            let buffer = match state.stdout_rx.recv_timeout(self.timeout) {
                Ok(l) => l,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    // Timeout occurred: kill the process and return error
                    // The next call will restart it via ensure_alive
                    if let Some(mut state) = self.state.take() {
                        let _ = state.child.kill();
                        let _ = state.child.wait();
                    }
                    return Err(anyhow!("Evaluation timed out after {:?}", self.timeout));
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    self.state.take();
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

    pub fn clear_namespace(&mut self, uri: &str) -> Result<()> {
        let state = self.ensure_alive()?;
        let req = serde_json::json!({
            "type": "clear-namespace",
            "uri": uri
        });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        state.stdin.write_all(line.as_bytes())?;
        state.stdin.flush()?;
        
        // Wait for READY to ensure the command was processed
        loop {
            let state = self.state.as_mut().unwrap();
            let buffer = state.stdout_rx.recv_timeout(self.timeout)?;
            if buffer.trim() == "READY" {
                break;
            }
        }
        Ok(())
    }

    pub fn parse(&mut self, target_path: &PathBuf) -> Result<Vec<RangeResult>> {
        let content = std::fs::read_to_string(target_path)?;
        let uri = format!("file:///{}", target_path.to_string_lossy());
        self.parse_str(&content, Some(&uri))
    }

    pub fn parse_str(&mut self, content: &str, uri: Option<&str>) -> Result<Vec<RangeResult>> {
        let state = self.ensure_alive()?;

        let req = serde_json::json!({
            "type": "parse",
            "content": content,
            "uri": uri
        });

        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        
        let mut retry = false;
        if state.stdin.write_all(line.as_bytes()).is_err() {
            retry = true;
        } else {
            let _ = state.stdin.flush();
        }

        if retry {
            self.state.take();
            let new_state = self.ensure_alive()?;
            new_state.stdin.write_all(line.as_bytes())?;
            new_state.stdin.flush()?;
        }

        let mut results = Vec::new();
        
        loop {
            let state = self.state.as_mut().unwrap();
            let buffer = match state.stdout_rx.recv_timeout(self.timeout) {
                Ok(l) => l,
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    if let Some(mut state) = self.state.take() {
                        let _ = state.child.kill();
                        let _ = state.child.wait();
                    }
                    return Err(anyhow!("Parsing timed out after {:?}", self.timeout));
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    self.state.take();
                    return Err(anyhow!("REPL process exited unexpectedly"));
                }
            };

            let trimmed = buffer.trim();
            if trimmed == "READY" {
                break;
            }
            
            if let Ok(res) = serde_json::from_str::<RangeResult>(trimmed) {
                results.push(res);
            }
        }

        Ok(results)
    }
}

impl Drop for Evaluator {
    fn drop(&mut self) {
        if let Some(mut state) = self.state.take() {
            let _ = state.child.kill();
            let _ = state.child.wait();
        }
        // Release the lock before attempting to delete the file
        drop(self._shim_lock.take());
        let _ = std::fs::remove_file(&self.shim_path);
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
        let mut evaluator = Evaluator::new().unwrap();
        // Set a very short timeout for the test
        evaluator.timeout = Duration::from_millis(500);

        // Infinite loop: (let loop () (loop))
        let result = evaluator.evaluate_str("(let loop () (loop))", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));

        // Verify recovery: subsequent evaluation should work (after restart)
        evaluator.timeout = Duration::from_secs(5);
        let result = evaluator.evaluate_str("42", None, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].result, "42");
    }

    #[test]
    fn test_delegated_parsing() {
        let mut evaluator = Evaluator::new().unwrap();
        evaluator.timeout = Duration::from_secs(5);

        let text = r#"
(define x 1)
#;
(define y 2)
#|
  Block comment
|#
(define z 3)
"#;
        let ranges = evaluator.parse_str(text, None).unwrap();
        
        // Should find (define x 1) and (define z 3), but skip (define y 2) and block comment
        assert_eq!(ranges.len(), 2);
        
        assert_eq!(ranges[0].line, 2);
        assert_eq!(ranges[0].col, 0);
        assert_eq!(ranges[0].end_line, 2);
        assert_eq!(ranges[0].end_col, 12);
        
        assert_eq!(ranges[1].line, 8);
        assert_eq!(ranges[1].col, 0);
        assert_eq!(ranges[1].end_line, 8);
        assert_eq!(ranges[1].end_col, 12);
    }

    #[test]
    fn test_per_document_isolation() {
        let mut evaluator = Evaluator::new().unwrap();
        
        // 1. Define x in doc A
        let res_a1 = evaluator.evaluate_str("(define x 42)", Some("file:///a.rkt"), None).unwrap();
        
        // 2. Access x in doc A (should succeed)
        let res_a2 = evaluator.evaluate_str("x", Some("file:///a.rkt"), None).unwrap();
        assert_eq!(res_a2[0].result, "42");
        
        // 3. Access x in doc B (should fail)
        let res_b1 = evaluator.evaluate_str("x", Some("file:///b.rkt"), None).unwrap();
        assert!(res_b1[0].is_error, "x should be undefined in document B");
        
        // 4. Clear doc A and access x (should fail)
        evaluator.clear_namespace("file:///a.rkt").unwrap();
        let res_a3 = evaluator.evaluate_str("x", Some("file:///a.rkt"), None).unwrap();
        assert!(res_a3[0].is_error, "x should be undefined in document A after clear");
    }
}
