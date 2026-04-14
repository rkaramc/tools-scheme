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
