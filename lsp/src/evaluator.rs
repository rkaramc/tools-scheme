use anyhow::{Result, anyhow};
use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const SHIM_SOURCE: &str = include_str!("eval-shim.rkt");
const TEMP_SUBDIR: &str = "vscode-scheme-toolbox-lsp";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    #[serde(default)]
    pub kind: String, // "code" or "markdown"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RangeResult {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    #[serde(default)]
    pub span: u32,
    #[serde(default)]
    pub pos: u32,
    #[serde(default)]
    pub kind: String, // "code" or "markdown"
    #[serde(default)]
    pub valid: bool,
}

#[derive(Debug)]
struct ProcessState {
    child: Child,
    stdin_tx: crossbeam_channel::Sender<String>,
    stdout_rx: Receiver<String>,
}

/// Manages process restart strategy with exponential backoff.
struct BackoffSupervisor {
    consecutive_failures: u32,
    last_attempt: Option<std::time::Instant>,
    min_delay: Duration,
    max_delay: Duration,
}

impl BackoffSupervisor {
    fn new(min_delay: Duration, max_delay: Duration) -> Self {
        Self {
            consecutive_failures: 0,
            last_attempt: None,
            min_delay,
            max_delay,
        }
    }

    /// Records a successful start, resetting failures.
    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.last_attempt = Some(std::time::Instant::now());
    }

    /// Records a failure.
    fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.last_attempt = Some(std::time::Instant::now());
    }

    /// Returns true if a restart attempt is allowed.
    fn can_restart(&self) -> bool {
        if self.consecutive_failures == 0 {
            return true;
        }

        if let Some(last) = self.last_attempt {
            let delay = self.current_delay();
            last.elapsed() >= delay
        } else {
            true
        }
    }

    /// Calculates current required delay based on failures.
    fn current_delay(&self) -> Duration {
        if self.consecutive_failures == 0 {
            return Duration::from_secs(0);
        }

        // exponential backoff: min * 2^(n-1)
        let exponent = (self.consecutive_failures - 1).min(31);
        let factor = 2u64.pow(exponent);
        let delay = self.min_delay * factor as u32;

        delay.min(self.max_delay)
    }

    #[allow(unused)]
    fn next_attempt_in(&self) -> Duration {
        if self.can_restart() {
            Duration::from_secs(0)
        } else if let Some(last) = self.last_attempt {
            let delay = self.current_delay();
            let elapsed = last.elapsed();
            if elapsed >= delay {
                Duration::from_secs(0)
            } else {
                delay - elapsed
            }
        } else {
            Duration::from_secs(0)
        }
    }
}

pub struct Evaluator {
    state: Option<ProcessState>,
    supervisor: BackoffSupervisor,
    shim_file: tempfile::NamedTempFile,
    timeout: Duration,
    global_session: std::fs::File,
    _global_session_path: PathBuf,
    racket_path: String,
    pending_cancellations: std::collections::HashSet<u32>,
}

impl Evaluator {
    pub fn new(racket_path: Option<String>) -> Result<Self> {
        let is_test = cfg!(test) || std::env::var("TOOLS_SCHEME_TEST").is_ok();

        let temp_dir = if let Ok(tmp) = std::env::var("TOOLS_SCHEME_TMP_DIR") {
            let p = std::path::PathBuf::from(tmp);
            if p.exists() {
                p
            } else {
                std::fs::create_dir_all(&p)?;
                p
            }
        } else if is_test {
            std::env::current_dir()?
        } else {
            let p = std::env::temp_dir().join(TEMP_SUBDIR);
            std::fs::create_dir_all(&p)?;
            p
        };

        // Use project-specific session name instead of random suffix, unless testing
        let session_name = if is_test {
            format!("test_{}.global.session.txt", fastrand::u32(..))
        } else {
            "global.session".to_string()
        };
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
            supervisor: BackoffSupervisor::new(Duration::from_secs(1), Duration::from_secs(60)),
            shim_file,
            timeout,
            global_session,
            _global_session_path: global_session_path,
            racket_path: final_racket_path,
            pending_cancellations: std::collections::HashSet::new(),
        })
    }

    #[allow(unused)]
    pub fn log(&self, msg: &str) {
        let mut file = &self.global_session;
        let _ = writeln!(file, "{}", msg);
        let _ = file.flush();
    }

    #[allow(unused)]
    pub fn racket_path(&self) -> &str {
        &self.racket_path
    }

    pub fn shutdown(&mut self) {
        if let Some(mut state) = self.state.take() {
            let _ = state.child.kill();
            let _ = state.child.wait();
        }
    }

    #[allow(unused)]
    pub fn session_path(&self) -> &Path {
        &self._global_session_path
    }

    #[allow(unused)]
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

        let mut cmd = if !has_separator {
            Command::new(path)
        } else {
            let p = Path::new(path);
            if !p.exists() {
                return Err(anyhow!("Racket path does not exist: {}", path));
            }
            if !p.is_file() {
                return Err(anyhow!("Racket path is not a file: {}", path));
            }
            Command::new(path)
        };

        let child_res = cmd
            .arg("--version")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn();

        let mut child = match child_res {
            Ok(c) => c,
            Err(e) => {
                writeln!(session_file, "Failed to execute Racket binary: {}", e)?;
                return Err(anyhow!(
                    "Failed to execute Racket binary at {}: {}",
                    path,
                    e
                ));
            }
        };

        let mut stdout_pipe = child.stdout.take().unwrap();
        let mut stderr_pipe = child.stderr.take().unwrap();

        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            use std::io::Read;
            let mut out = Vec::new();
            let mut err = Vec::new();
            let _ = stdout_pipe.read_to_end(&mut out);
            let _ = stderr_pipe.read_to_end(&mut err);
            let _ = tx.send((out, err));
        });

        let start = std::time::Instant::now();
        let mut status = None;
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(s)) = child.try_wait() {
                status = Some(s);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        if status.is_none() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!(
                "Validation timed out waiting for Racket binary at {}",
                path
            ));
        }

        let status = status.unwrap();
        let (stdout_bytes, stderr_bytes) =
            rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
        let stdout = String::from_utf8_lossy(&stdout_bytes);
        let stderr = String::from_utf8_lossy(&stderr_bytes);

        writeln!(session_file, "Racket --version status: {}", status)?;
        writeln!(session_file, "Racket --version stdout: {}", stdout.trim())?;
        if !stderr.is_empty() {
            writeln!(session_file, "Racket --version stderr: {}", stderr.trim())?;
        }

        if !status.success() {
            return Err(anyhow!(
                "Racket binary at {} failed with exit code {}. Stderr: {}",
                path,
                status,
                stderr.trim()
            ));
        }

        if !stdout.contains("Racket") && !stderr.contains("Racket") {
            return Err(anyhow!(
                "Binary at {} does not appear to be Racket. Output: {}",
                path,
                stdout.trim()
            ));
        }

        writeln!(session_file, "Racket path validation successful.")?;
        Ok(())
    }

    fn spawn_process(
        racket_path: &str,
        shim_path: &Path,
        session_file: &File,
    ) -> Result<ProcessState> {
        let mut child = Command::new(racket_path)
            .arg(shim_path)
            .arg("--repl")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(session_file.try_clone()?))
            .spawn()?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open stdout"))?;

        let (stdin_tx, stdin_rx) = crossbeam_channel::bounded::<String>(100);
        std::thread::spawn(move || {
            for msg in stdin_rx {
                if stdin.write_all(msg.as_bytes()).is_err() {
                    break;
                }
                if stdin.flush().is_err() {
                    break;
                }
            }
        });

        // Bounded channel for REPL stdout to prevent memory explosion if REPL
        // sends huge amounts of data. The worker thread reads this via recv_timeout.
        let (tx, rx) = crossbeam_channel::bounded(100);

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
            stdin_tx,
            stdout_rx: rx,
        })
    }

    fn ensure_alive(&mut self) -> Result<&mut ProcessState> {
        let needs_restart = match &mut self.state {
            Some(state) => match state.child.try_wait() {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(_) => true,
            },
            None => true,
        };

        if needs_restart {
            if !self.supervisor.can_restart() {
                let next = self.supervisor.next_attempt_in();
                return Err(anyhow!(
                    "Racket process is dead and restart is throttled. Try again in {}s",
                    next.as_secs()
                ));
            }

            // Drop old state explicitly
            if let Some(mut old_state) = self.state.take() {
                let _ = old_state.child.kill();
                let _ = old_state.child.wait();
            }

            match Self::spawn_process(
                &self.racket_path,
                self.shim_file.path(),
                &self.global_session,
            ) {
                Ok(new_state) => {
                    self.supervisor.record_success();
                    self.state = Some(new_state);
                }
                Err(e) => {
                    self.supervisor.record_failure();
                    return Err(anyhow!("Failed to restart Racket process: {}", e));
                }
            }
        }

        Ok(self
            .state
            .as_mut()
            .ok_or_else(|| anyhow!("Racket process unavailable (throttled)"))?)
    }

    fn send_command<F>(
        &mut self,
        req: &serde_json::Value,
        cancel_info: Option<(&crossbeam_channel::Receiver<u32>, u32, &str)>,
        log_file: Option<&File>,
        on_line: F,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        // 1. Check if this task was already cancelled before we even started
        if let Some((_, id, _)) = cancel_info {
            if self.pending_cancellations.remove(&id) {
                return Err(anyhow!("Task {} was cancelled before start", id));
            }
        }

        // 2. Ensure REPL is alive and get channels
        let (stdin_tx, stdout_rx) = {
            let state = self.ensure_alive()?;
            (state.stdin_tx.clone(), state.stdout_rx.clone())
        };

        let mut line = serde_json::to_string(req)?;
        line.push('\n');

        if stdin_tx.send(line.clone()).is_err() {
            // Retry once if channel is broken (process might have died just now)
            self.state.take();
            let (new_stdin_tx, new_stdout_rx) = {
                let state = self.ensure_alive()?;
                (state.stdin_tx.clone(), state.stdout_rx.clone())
            };
            new_stdin_tx.send(line)?;
            // Update our local channels to the new ones
            // Wait, we need to keep using the new ones in the loop
            return self.send_command_loop(
                new_stdin_tx,
                new_stdout_rx,
                cancel_info,
                log_file,
                on_line,
            );
        }

        self.send_command_loop(stdin_tx, stdout_rx, cancel_info, log_file, on_line)
    }

    fn send_command_loop<F>(
        &mut self,
        stdin_tx: crossbeam_channel::Sender<String>,
        stdout_rx: Receiver<String>,
        cancel_info: Option<(&crossbeam_channel::Receiver<u32>, u32, &str)>,
        log_file: Option<&File>,
        mut on_line: F,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        loop {
            let buffer = if let Some((rx, id, uri)) = cancel_info {
                crossbeam_channel::select! {
                    recv(stdout_rx) -> msg => match msg {
                        Ok(l) => l,
                        Err(_) => {
                            self.state.take();
                            return Err(anyhow!("REPL process exited unexpectedly"));
                        }
                    },
                    recv(rx) -> msg => {
                        if let Ok(cancel_id) = msg {
                            if cancel_id == id {
                                let cancel_req = serde_json::json!({
                                    "type": "cancel-evaluation",
                                    "uri": uri
                                });
                                let mut cancel_line = serde_json::to_string(&cancel_req).unwrap();
                                cancel_line.push('\n');
                                let _ = stdin_tx.send(cancel_line);
                            } else {
                                // Store cancellation for a future task so it's not lost
                                self.pending_cancellations.insert(cancel_id);
                            }
                        }
                        // Wait for the next message
                        continue;
                    }
                    default(self.timeout) => {
                        if let Some(mut state) = self.state.take() {
                            let _ = state.child.kill();
                            let _ = state.child.wait();
                        }
                        return Err(anyhow!("Command timed out after {:?}", self.timeout));
                    }
                }
            } else {
                match stdout_rx.recv_timeout(self.timeout) {
                    Ok(l) => l,
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                        if let Some(mut state) = self.state.take() {
                            let _ = state.child.kill();
                            let _ = state.child.wait();
                        }
                        return Err(anyhow!("Command timed out after {:?}", self.timeout));
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        self.state.take();
                        return Err(anyhow!("REPL process exited unexpectedly"));
                    }
                }
            };

            let trimmed = buffer.trim();
            if trimmed == "READY" {
                break;
            }

            on_line(&buffer);

            if let Some(mut file) = log_file {
                let _ = writeln!(file, "{}", buffer);
                let _ = file.flush();
            } else {
                let _ = writeln!(&mut self.global_session, "{}", buffer);
                let _ = self.global_session.flush();
            }
        }

        Ok(())
    }

    #[allow(unused)]
    pub fn evaluate(&mut self, target_path: &PathBuf) -> Result<Vec<EvalResult>> {
        let content = std::fs::read_to_string(target_path)?;
        let uri = format!("file:///{}", target_path.to_string_lossy());
        let context_label = target_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        self.evaluate_str(&content, Some(&uri), context_label.as_deref(), None)
    }

    pub fn evaluate_str(
        &mut self,
        content: &str,
        uri: Option<&str>,
        context_label: Option<&str>,
        log: Option<&File>,
    ) -> Result<Vec<EvalResult>> {
        let label = context_label.or(uri).unwrap_or("UNKNOWN");

        if let Some(mut file) = log {
            writeln!(
                file,
                "\n--- EVAL INPUT ({}) ---\n{}\n--- EVAL OUTPUT ---",
                label, content
            )?;
            file.flush()?;
        } else {
            writeln!(
                &mut self.global_session,
                "\n--- EVAL INPUT NO LOG ({}) ---\n{}\n--- EVAL OUTPUT ---",
                label, content
            )?;
            self.global_session.flush()?;
        }

        let req = serde_json::json!({
            "type": "evaluate",
            "content": content,
            "uri": uri
        });

        let mut results = Vec::new();

        self.send_command(&req, None, log, |buffer| {
            let trimmed = buffer.trim();
            if let Ok(res) = serde_json::from_str::<EvalResult>(trimmed) {
                results.push(res);
            }
        })?;

        Ok(results)
    }

    pub fn parse(&mut self, content: &str, uri: Option<&str>) -> Result<Vec<RangeResult>> {
        let req = serde_json::json!({
            "type": "parse",
            "content": content,
            "uri": uri
        });

        let mut results = Vec::new();
        self.send_command(&req, None, None, |trimmed| {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if json_val.get("type").and_then(|v| v.as_str()) == Some("range") {
                    if let Ok(res) = serde_json::from_value::<RangeResult>(json_val) {
                        results.push(res);
                    }
                }
            }
        })?;

        Ok(results)
    }

    #[allow(unused)]
    pub fn clear_namespace(&mut self, uri: &str, log: Option<&File>) -> Result<()> {
        if let Some(mut file) = log {
            writeln!(file, "\n--- SYSTEM COMMAND: clear-namespace ({}) ---", uri)?;
            file.flush()?;
        } else {
            writeln!(
                &mut self.global_session,
                "\n--- SYSTEM COMMAND: clear-namespace ({}) ---",
                uri
            )?;
            self.global_session.flush()?;
        }

        let req = serde_json::json!({
            "type": "clear-namespace",
            "uri": uri
        });
        self.send_command(&req, None, log, |_| {})
    }

    pub fn get_rich_media(&mut self, id: &str, log: Option<&File>) -> Result<String> {
        let req = serde_json::json!({
            "type": "get-rich-media",
            "id": id
        });

        let mut data = String::new();
        self.send_command(&req, None, log, |trimmed| {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if json_val.get("type").and_then(|v| v.as_str()) == Some("rich-data") {
                    if let Some(d) = json_val.get("data").and_then(|v| v.as_str()) {
                        data = d.to_string();
                    }
                }
            }
        })?;

        Ok(data)
    }

    pub fn validate_blocks(
        &mut self,
        blocks: Vec<String>,
        log: Option<&File>,
    ) -> Result<Vec<bool>> {
        if let Some(mut file) = log {
            writeln!(
                file,
                "\n--- SYSTEM COMMAND: validate-blocks ({} blocks) ---",
                blocks.len()
            )?;
            file.flush()?;
        } else {
            writeln!(
                &mut self.global_session,
                "\n--- SYSTEM COMMAND: validate-blocks ({} blocks) ---",
                blocks.len()
            )?;
            self.global_session.flush()?;
        }

        let req = serde_json::json!({
            "type": "validate-blocks",
            "blocks": blocks
        });

        let mut results = vec![false; blocks.len()];

        self.send_command(&req, None, log, |trimmed| {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if json_val.get("type").and_then(|v| v.as_str()) == Some("validation") {
                    let index =
                        json_val.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    let valid = json_val
                        .get("valid")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if index < results.len() {
                        results[index] = valid;
                    }
                }
            }
        })?;

        Ok(results)
    }

    pub fn evaluate_notebook_cell<F>(
        &mut self,
        content: &str,
        uri: &str,
        cancel_rx: &crossbeam_channel::Receiver<u32>,
        execution_id: u32,
        mut on_line: F,
    ) -> Result<()>
    where
        F: FnMut(&str),
    {
        writeln!(
            &mut self.global_session,
            "\n--- EVAL CELL INPUT NO LOG ({}) ---\n{}\n--- EVAL CELL OUTPUT ---",
            uri, content
        )?;
        self.global_session.flush()?;

        let req = serde_json::json!({
            "type": "evaluate",
            "content": content,
            "uri": uri
        });

        self.send_command(&req, Some((cancel_rx, execution_id, uri)), None, |buffer| {
            on_line(buffer.trim());
        })
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
        let mut reader = BufReader::new(stdout);
        let mut buffer = String::new();

        while reader.read_line(&mut buffer)? > 0 {
            if let Ok(res) = serde_json::from_str::<EvalResult>(&buffer) {
                results.push(res);
            }
            buffer.clear();
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
        let json =
            r#"{"line":1,"col":10,"result":"void","is_error":false,"output":"hello\nworld"}"#;
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
    fn test_per_document_isolation() {
        let mut evaluator = Evaluator::new(None).unwrap();

        // 1. Define x in doc A
        let res_a1 = evaluator
            .evaluate_str("(define x 42)", Some("file:///a.rkt"), None, None)
            .unwrap();
        assert!(res_a1.is_empty());

        // 2. Access x in doc A (should succeed)
        let res_a2 = evaluator
            .evaluate_str("x", Some("file:///a.rkt"), None, None)
            .unwrap();
        assert_eq!(res_a2[0].result, "42");

        // 3. Access x in doc B (should fail)
        let res_b1 = evaluator
            .evaluate_str("x", Some("file:///b.rkt"), None, None)
            .unwrap();
        assert!(res_b1[0].is_error, "x should be undefined in document B");

        // 4. Clear doc A and access x (should fail)
        evaluator.clear_namespace("file:///a.rkt", None).unwrap();
        let res_a3 = evaluator
            .evaluate_str("x", Some("file:///a.rkt"), None, None)
            .unwrap();
        assert!(
            res_a3[0].is_error,
            "x should be undefined in document A after clear"
        );
    }

    #[test]
    fn test_syntax_recovery() {
        let mut evaluator = Evaluator::new(None).unwrap();
        evaluator.timeout = Duration::from_secs(5);

        let code = "1\n(unclosed-bracket\n2";
        let results = evaluator
            .evaluate_str(code, Some("file:///test.rkt"), None, None)
            .unwrap();

        let has_1 = results.iter().any(|r| r.result == "1");
        let has_error = results.iter().any(|r| r.is_error);
        let has_2 = results.iter().any(|r| r.result == "2");

        assert!(has_1, "Should have evaluated 1. Results: {:?}", results);
        assert!(
            has_error,
            "Should have reported syntax error. Results: {:?}",
            results
        );
        assert!(
            has_2,
            "Should have recovered and evaluated 2. Results: {:?}",
            results
        );
    }

    #[test]
    fn test_syntax_recovery_complex() {
        let mut evaluator = Evaluator::new(None).unwrap();
        evaluator.timeout = Duration::from_secs(5);

        let code = "1\n(define \n(error\n2";
        let results = evaluator
            .evaluate_str(code, Some("file:///test.rkt"), None, None)
            .unwrap();

        println!("RESULTS: {:#?}", results);

        let has_1 = results.iter().any(|r| r.result == "1");
        let has_2 = results.iter().any(|r| r.result == "2");
        let error_count = results.iter().filter(|r| r.is_error).count();

        assert!(has_1, "Should have evaluated 1");
        assert!(has_2, "Should have evaluated 2");
        assert!(
            error_count >= 2,
            "Should have reported at least two errors (got {})",
            error_count
        );
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
                assert!(
                    err.contains("does not appear to be Racket")
                        || err.contains("failed with exit code")
                        || err.contains("Failed to execute Racket binary")
                );
            }
        }
    }

    #[test]
    fn test_validate_racket_path_timeout() {
        let code = r#"
            fn main() {
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        "#;
        let mut path = std::env::temp_dir();
        path.push("dummy_racket.rs");
        std::fs::write(&path, code).unwrap();

        let exe_path = path.with_extension(std::env::consts::EXE_EXTENSION);
        let status = std::process::Command::new("rustc")
            .arg(&path)
            .arg("-o")
            .arg(&exe_path)
            .status()
            .unwrap();
        assert!(status.success(), "Failed to compile dummy_racket.rs");

        let start = std::time::Instant::now();
        let res = Evaluator::new(Some(exe_path.to_string_lossy().to_string()));
        let duration = start.elapsed();

        match res {
            Ok(_) => panic!("Evaluator should fail when validation binary hangs"),
            Err(e) => {
                let err = e.to_string();
                println!("ACTUAL TIMEOUT ERROR: {}", err);
                assert!(
                    err.contains("timed out"),
                    "Error should mention timeout: {}",
                    err
                );
                assert!(
                    duration.as_secs() < 10,
                    "Validation should timeout quickly, not hang"
                );
            }
        }
    }

    #[test]
    fn test_invalid_racket_path() {
        // This should fail with our new validation
        let res = Evaluator::new(Some("non-existent-racket-binary-XYZ".to_string()));
        assert!(
            res.is_err(),
            "Evaluator should fail with invalid racket path"
        );
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
        evaluator
            .evaluate_str("(define x 42)", Some("file:///test.rkt"), None, None)
            .unwrap();

        // 2. Verify x exists
        let res = evaluator
            .evaluate_str("x", Some("file:///test.rkt"), None, None)
            .unwrap();
        assert_eq!(res[0].result, "42");

        // 3. Restart
        evaluator.restart().unwrap();

        // 4. Verify x is gone
        let res = evaluator
            .evaluate_str("x", Some("file:///test.rkt"), None, None)
            .unwrap();
        assert!(
            res[0].is_error,
            "x should be undefined after restart. Result: {:?}",
            res
        );
    }

    #[test]
    fn test_global_session_location() {
        let local_session = std::path::Path::new("global.session");
        // Cleanup if exists
        let _ = std::fs::remove_file(local_session);

        let _evaluator = Evaluator::new(None).unwrap();

        assert!(
            !local_session.exists(),
            "global.session should NOT be created in the current directory"
        );
    }

    #[test]
    fn test_lang_recursive_function_no_namespace_mismatch() {
        // Reproduces ts-h31: a recursive define in a #lang racket file produces
        // "namespace mismatch; cannot locate module instance" errors on the
        // self-reference call sites when evaluate-port pre-expands the module
        // and then evals each body form individually.
        let mut repro_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        repro_dir.push("repro");

        let path = repro_dir.join("repro_ts_h31.rkt");
        let content =
            std::fs::read_to_string(&path).expect("repro_ts_h31.rkt not found in lsp/repro/");

        let path_str = path.to_string_lossy().replace('\\', "/");
        let uri = format!("file:///{}", path_str);

        let mut evaluator = Evaluator::new(None).unwrap();
        evaluator.timeout = Duration::from_secs(15);

        let results = evaluator
            .evaluate_str(&content, Some(&uri), Some("repro_ts_h31.rkt"), None)
            .unwrap();

        let namespace_errors: Vec<&str> = results
            .iter()
            .filter(|r| r.is_error && r.result.contains("namespace mismatch"))
            .map(|r| r.result.as_str())
            .collect();

        assert!(
            namespace_errors.is_empty(),
            "Got namespace mismatch errors (ts-h31 not fixed):\n{:#?}",
            namespace_errors
        );

        // (lat? '(a b c)) => #t, (lat? '(a (b) c)) => #f
        let values: Vec<&str> = results.iter().map(|r| r.result.as_str()).collect();
        assert!(
            values.contains(&"#t"),
            "Expected (lat? '(a b c)) = #t, got: {:?}",
            values
        );
        assert!(
            values.contains(&"#f"),
            "Expected (lat? '(a (b) c)) = #f, got: {:?}",
            values
        );
    }

    #[test]
    fn test_relative_require_resolves_to_file_directory() {
        // Reproduces ts-k2w: relative (require "...") in a #lang file resolves
        // against the shim's CWD instead of the file's directory.
        //
        // repro_require_main.rkt does (require "repro_require_helper.rkt")
        // where both files live in lsp/repro/.  Passing the real file URI
        // should make the shim set current-directory to that folder so the
        // require succeeds.
        let mut repro_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        repro_dir.push("repro");

        let main_path = repro_dir.join("repro_require_main.rkt");
        let content = std::fs::read_to_string(&main_path)
            .expect("repro_require_main.rkt not found in lsp/repro/");

        // Build a file:/// URI matching the real path so the shim can extract
        // the directory.  Use forward slashes; percent-encode the colon on
        // Windows to match what VS Code sends.
        let path_str = main_path.to_string_lossy().replace('\\', "/");
        let uri = format!("file:///{}", path_str);

        let mut evaluator = Evaluator::new(None).unwrap();
        evaluator.timeout = Duration::from_secs(15);

        let results = evaluator
            .evaluate_str(&content, Some(&uri), Some("repro_require_main.rkt"), None)
            .unwrap();

        let has_require_error = results
            .iter()
            .any(|r| r.is_error && r.result.contains("cannot open module file"));

        assert!(
            !has_require_error,
            "Relative require failed — current-directory not set to file directory.\n\
             Results: {:#?}",
            results
        );

        // After the fix (square 5) → 25 and (square 12) → 144 should appear.
        let values: Vec<&str> = results.iter().map(|r| r.result.as_str()).collect();
        assert!(
            values.contains(&"25"),
            "Expected (square 5) = 25, got: {:?}",
            values
        );
        assert!(
            values.contains(&"144"),
            "Expected (square 12) = 144, got: {:?}",
            values
        );
    }

    #[test]
    fn test_backoff_supervisor_success_resets() {
        let mut supervisor =
            BackoffSupervisor::new(Duration::from_millis(100), Duration::from_secs(1));
        supervisor.record_failure();
        assert_eq!(supervisor.consecutive_failures, 1);
        supervisor.record_success();
        assert_eq!(supervisor.consecutive_failures, 0);
    }

    #[test]
    fn test_backoff_supervisor_exponential_delay() {
        let min = Duration::from_millis(100);
        let max = Duration::from_secs(1);
        let mut supervisor = BackoffSupervisor::new(min, max);

        supervisor.record_failure(); // 1st failure: delay = min * 2^0 = 100ms
        assert_eq!(supervisor.current_delay(), min);

        supervisor.record_failure(); // 2nd failure: delay = min * 2^1 = 200ms
        assert_eq!(supervisor.current_delay(), min * 2);

        supervisor.record_failure(); // 3rd failure: delay = min * 2^2 = 400ms
        assert_eq!(supervisor.current_delay(), min * 4);

        for _ in 0..10 {
            supervisor.record_failure();
        }
        assert_eq!(supervisor.current_delay(), max); // capped at max
    }

    #[test]
    fn test_backoff_supervisor_throttling() {
        let mut supervisor =
            BackoffSupervisor::new(Duration::from_millis(100), Duration::from_secs(1));

        assert!(
            supervisor.can_restart(),
            "Initial attempt should be allowed"
        );

        supervisor.record_failure();
        assert!(
            !supervisor.can_restart(),
            "Immediate retry should be throttled"
        );

        std::thread::sleep(Duration::from_millis(150));
        assert!(
            supervisor.can_restart(),
            "Retry after delay should be allowed"
        );
    }

    #[test]
    fn test_evaluator_backoff_on_spawn_failure() {
        // Use an invalid binary that doesn't exist to cause spawn failure
        // We'll have to skip the validation to get past new()
        // Wait, Evaluator::new() calls validate_racket_path and spawn_process.
        // If they fail, new() fails.

        // To test backoff, we need an Evaluator that is ALREADY created but then
        // its process dies and it fails to restart.
        let mut evaluator = Evaluator::new(None).unwrap();

        // Break the racket path so subsequent restarts fail
        evaluator.racket_path = "non-existent-racket-binary-XYZ".to_string();

        // Kill the current process
        if let Some(mut state) = evaluator.state.take() {
            let _ = state.child.kill();
            let _ = state.child.wait();
        }

        // First attempt to ensure_alive should fail and record a failure
        let res1 = evaluator.ensure_alive();
        assert!(res1.is_err(), "First restart attempt should fail");
        assert!(
            res1.as_ref()
                .unwrap_err()
                .to_string()
                .contains("Failed to restart Racket process")
        );

        // Second attempt should be throttled immediately
        let res2 = evaluator.ensure_alive();
        assert!(res2.is_err(), "Second restart attempt should be throttled");
        assert!(
            res2.as_ref()
                .unwrap_err()
                .to_string()
                .contains("restart is throttled"),
            "Error was: {:?}",
            res2
        );
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

        evaluator
            .evaluate_str("(define x 1)", Some(uri), Some("test.rkt"), Some(&file))
            .unwrap();

        let log_content = std::fs::read_to_string(&log_path).unwrap();

        assert!(log_content.contains("EVAL INPUT (test.rkt)"));

        let _ = std::fs::remove_file(&log_path);
    }
}
