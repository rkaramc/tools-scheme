use serde::{Deserialize, Serialize};
use std::process::{Command, Stdio, Child, ChildStdin};
use std::io::{BufRead, BufReader, Write};
use anyhow::{Result, anyhow};
use std::path::{PathBuf, Path};
use std::fs::File;
use std::time::Duration;
use crossbeam_channel::Receiver;

const SHIM_SOURCE: &str = include_str!("eval-shim.rkt");
const TEMP_SUBDIR: &str = "vscode-scheme-toolbox-lsp";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub line: u32,
    pub col: u32,
    #[serde(default)]
    pub end_line: u32,
    #[serde(default)]
    pub end_col: u32,
    #[serde(default)]
    pub span: u32,
    #[serde(default)]
    pub pos: u32,
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
    #[serde(default)]
    pub span: u32,
    #[serde(default)]
    pub pos: u32,
}

struct ProcessState {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
}

pub struct Evaluator {
    state: Option<ProcessState>,
    shim_file: tempfile::NamedTempFile,
    timeout: Duration,
    global_session: std::fs::File,
    global_session_path: PathBuf,
    racket_path: String,
}

impl Evaluator {
    pub fn new(racket_path: Option<String>) -> Result<Self> {
        let temp_dir = std::env::temp_dir().join(TEMP_SUBDIR);
        std::fs::create_dir_all(&temp_dir)?;

        let random_suffix: String = std::iter::repeat_with(|| fastrand::lowercase()).take(8).collect();
        let session_name = format!("global-{}.session", random_suffix);
        let global_session_path = temp_dir.join(session_name);

        let mut global_session = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&global_session_path)?;

        let timeout_secs = std::env::var("TOOLS_SCHEME_EVAL_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(15);
        let timeout = Duration::from_secs(timeout_secs);

        let final_racket_path = racket_path
            .or_else(|| std::env::var("TOOLS_SCHEME_RACKET_PATH").ok())
            .unwrap_or_else(|| "racket".to_string());

        // Validate racket path
        Self::validate_racket_path(&final_racket_path, &mut global_session)?;

        // Prepare the embedded shim in a secure temporary location
        let mut shim_file = tempfile::Builder::new()
            .prefix("eval-shim-")
            .suffix(".rkt")
            .tempfile_in(&temp_dir)?;
        
        shim_file.write_all(SHIM_SOURCE.as_bytes())?;
        shim_file.flush()?;

        let state = Self::spawn_process(&final_racket_path, shim_file.path(), &global_session)?;

        Ok(Self {
            state: Some(state),
            shim_file: shim_file,
            timeout,
            global_session,
            global_session_path,
            racket_path: final_racket_path,
        })
    }

    pub fn racket_path(&self) -> &str {
        &self.racket_path
    }

    pub fn session_path(&self) -> &Path {
        &self.global_session_path
    }

    pub fn restart(&mut self) -> Result<()> {
        if let Some(mut state) = self.state.take() {
            let _ = state.child.kill();
            let _ = state.child.wait();
        }
        self.ensure_alive()?;
        Ok(())
    }

    fn validate_racket_path(path: &str, session_file: &mut File) -> Result<()> {
        writeln!(session_file, "Validating Racket path: {}", path)?;
        
        let has_separator = path.contains('/') || path.contains('\\');
        
        let output = if !has_separator {
            // Assume it's a binary in the PATH
            Command::new(path).arg("--version").output()
        } else {
            let p = Path::new(path);
            if !p.exists() {
                return Err(anyhow!("Racket path does not exist: {}", path));
            }
            if !p.is_file() {
                return Err(anyhow!("Racket path is not a file: {}", path));
            }
            Command::new(path).arg("--version").output()
        };

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                
                writeln!(session_file, "Racket --version status: {}", out.status)?;
                writeln!(session_file, "Racket --version stdout: {}", stdout.trim())?;
                if !stderr.is_empty() {
                    writeln!(session_file, "Racket --version stderr: {}", stderr.trim())?;
                }

                if !out.status.success() {
                    return Err(anyhow!("Racket binary at {} failed with exit code {}. Stderr: {}", path, out.status, stderr.trim()));
                }

                if !stdout.contains("Racket") && !stderr.contains("Racket") {
                    return Err(anyhow!("Binary at {} does not appear to be Racket. Output: {}", path, stdout.trim()));
                }

                writeln!(session_file, "Racket path validation successful.")?;
                Ok(())
            }
            Err(e) => {
                writeln!(session_file, "Failed to execute Racket binary: {}", e)?;
                Err(anyhow!("Failed to execute Racket binary at {}: {}", path, e))
            }
        }
    }

    fn spawn_process(racket_path: &str, shim_path: &Path, session_file: &File) -> Result<ProcessState> {
        let mut child = Command::new(racket_path)
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
            self.state = Some(Self::spawn_process(&self.racket_path, self.shim_file.path(), &self.global_session)?);
        }
        
        Ok(self.state.as_mut().unwrap())
    }

    #[allow(unused)]
    pub fn evaluate(&mut self, target_path: &PathBuf) -> Result<Vec<EvalResult>> {
        let content = std::fs::read_to_string(target_path)?;
        let uri = format!("file:///{}", target_path.to_string_lossy());
        let context_label = target_path.file_name().map(|n| n.to_string_lossy().into_owned());
        self.evaluate_str(&content, Some(&uri), context_label.as_deref(), None)
    }

    pub fn evaluate_str(&mut self, content: &str, uri: Option<&str>, context_label: Option<&str>, log: Option<&File>) -> Result<Vec<EvalResult>> {
        if let Some(mut file) = log {
            if let Some(label) = context_label {
                writeln!(file, "\n--- EVAL INPUT ({}) ---\n{}\n--- EVAL OUTPUT ---", label, content)?;
            } else {
                writeln!(file, "\n--- EVAL INPUT ---\n{}\n--- EVAL OUTPUT ---", content)?;
            }
            file.flush()?;
        } else {
            let label = context_label.or(uri).unwrap_or("UNKNOWN");
            writeln!(&mut self.global_session, "\n--- EVAL INPUT NO LOG ({}) ---\n{}\n--- EVAL OUTPUT ---", label, content)?;
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

    #[allow(unused)]
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

    #[allow(unused)]
    pub fn parse(&mut self, target_path: &PathBuf) -> Result<Vec<RangeResult>> {
        let content = std::fs::read_to_string(target_path)?;
        let uri = format!("file:///{}", target_path.to_string_lossy());
        self.parse_str(&content, Some(&uri))
    }

    #[allow(unused)]
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
        let mut evaluator = Evaluator::new(None).unwrap();
        // Set a very short timeout for the test
        evaluator.timeout = Duration::from_millis(500);

        // Infinite loop: (let loop () (loop))
        let result = evaluator.evaluate_str("(let loop () (loop))", None, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));

        // Verify recovery: subsequent evaluation should work (after restart)
        evaluator.timeout = Duration::from_secs(5);
        let result = evaluator.evaluate_str("42", None, None, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].result, "42");
    }

    #[test]
    fn test_delegated_parsing() {
        let mut evaluator = Evaluator::new(None).unwrap();
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
        let mut evaluator = Evaluator::new(None).unwrap();
        
        // 1. Define x in doc A
        let res_a1 = evaluator.evaluate_str("(define x 42)", Some("file:///a.rkt"), None, None).unwrap();
        assert!(res_a1.is_empty());
        
        // 2. Access x in doc A (should succeed)
        let res_a2 = evaluator.evaluate_str("x", Some("file:///a.rkt"), None, None).unwrap();
        assert_eq!(res_a2[0].result, "42");
        
        // 3. Access x in doc B (should fail)
        let res_b1 = evaluator.evaluate_str("x", Some("file:///b.rkt"), None, None).unwrap();
        assert!(res_b1[0].is_error, "x should be undefined in document B");
        
        // 4. Clear doc A and access x (should fail)
        evaluator.clear_namespace("file:///a.rkt").unwrap();
        let res_a3 = evaluator.evaluate_str("x", Some("file:///a.rkt"), None, None).unwrap();
        assert!(res_a3[0].is_error, "x should be undefined in document A after clear");
    }

    #[test]
    fn test_syntax_recovery() {
        let mut evaluator = Evaluator::new(None).unwrap();
        evaluator.timeout = Duration::from_secs(5);

        let code = "1\n(unclosed-bracket\n2";
        let results = evaluator.evaluate_str(code, Some("file:///test.rkt"), None, None).unwrap();
        
        let has_1 = results.iter().any(|r| r.result == "1");
        let has_error = results.iter().any(|r| r.is_error);
        let has_2 = results.iter().any(|r| r.result == "2");

        assert!(has_1, "Should have evaluated 1. Results: {:?}", results);
        assert!(has_error, "Should have reported syntax error. Results: {:?}", results);
        assert!(has_2, "Should have recovered and evaluated 2. Results: {:?}", results);
    }

    #[test]
    fn test_syntax_recovery_complex() {
        let mut evaluator = Evaluator::new(None).unwrap();
        evaluator.timeout = Duration::from_secs(5);

        let code = "1\n(define \n(error\n2";
        let results = evaluator.evaluate_str(code, Some("file:///test.rkt"), None, None).unwrap();
        
        println!("RESULTS: {:#?}", results);
        
        let has_1 = results.iter().any(|r| r.result == "1");
        let has_2 = results.iter().any(|r| r.result == "2");
        let error_count = results.iter().filter(|r| r.is_error).count();

        assert!(has_1, "Should have evaluated 1");
        assert!(has_2, "Should have evaluated 2");
        assert!(error_count >= 2, "Should have reported at least two errors (got {})", error_count);
    }

    #[test]
    fn test_not_racket_binary() {
        // Use a common system binary that is NOT racket
        let binary = if cfg!(windows) { "ping.exe" } else { "ls" };
        let res = Evaluator::new(Some(binary.to_string()));
        match res {
            Ok(_) => panic!("Evaluator should have failed with non-racket binary"),
            Err(e) => {
                let err = e.to_string();
                println!("ACTUAL ERROR: {}", err);
                assert!(err.contains("does not appear to be Racket") || 
                        err.contains("failed with exit code") ||
                        err.contains("Failed to execute Racket binary"));
            }
        }
    }

    #[test]
    fn test_invalid_racket_path() {
        // This should fail with our new validation
        let res = Evaluator::new(Some("non-existent-racket-binary-XYZ".to_string()));
        assert!(res.is_err(), "Evaluator should fail with invalid racket path");
    }

    #[test]
    fn test_racket_path_resolution() {
        // 1. Default (uses "racket")
        let ev_default = Evaluator::new(None).unwrap();
        assert_eq!(ev_default.racket_path, "racket");
    }

    #[test]
    fn test_restart_clears_state() {
        let mut evaluator = Evaluator::new(None).unwrap();
        
        // 1. Define x
        evaluator.evaluate_str("(define x 42)", Some("file:///test.rkt"), None, None).unwrap();
        
        // 2. Verify x exists
        let res = evaluator.evaluate_str("x", Some("file:///test.rkt"), None, None).unwrap();
        assert_eq!(res[0].result, "42");
        
        // 3. Restart
        evaluator.restart().unwrap();
        
        // 4. Verify x is gone
        let res = evaluator.evaluate_str("x", Some("file:///test.rkt"), None, None).unwrap();
        assert!(res[0].is_error, "x should be undefined after restart. Result: {:?}", res);
    }

    #[test]
    fn test_global_session_location() {
        let local_session = std::path::Path::new("global.session");
        // Cleanup if exists
        let _ = std::fs::remove_file(local_session);

        let _evaluator = Evaluator::new(None).unwrap();
        
        assert!(!local_session.exists(), "global.session should NOT be created in the current directory");
    }

    #[test]
    fn test_evaluate_str_logging() {
        let mut evaluator = Evaluator::new(None).unwrap();
        let temp_dir = std::env::temp_dir();
        let log_path = temp_dir.join("test_eval_log.session");
        
        // Ensure file is clean
        let _ = std::fs::remove_file(&log_path);
        
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
            .unwrap();

        let uri = if cfg!(windows) {
            "file:///C:/path/to/test.rkt"
        } else {
            "file:///path/to/test.rkt"
        };
        
        evaluator.evaluate_str("(define x 1)", Some(uri), Some("test.rkt"), Some(&file)).unwrap();
        
        let log_content = std::fs::read_to_string(&log_path).unwrap();
        
        assert!(log_content.contains("EVAL INPUT (test.rkt)"));

        let _ = std::fs::remove_file(&log_path);
    }
}
