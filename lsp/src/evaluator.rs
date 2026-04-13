use serde::{Deserialize, Serialize};
use std::process::Command;
use std::io::{BufRead, BufReader};
use anyhow::{Result, anyhow};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub line: u32,
    pub col: u32,
    pub result: String,
    pub is_error: bool,
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

        let mut results = Vec::new();
        let reader = BufReader::new(&output.stdout[..]);
        for line in reader.lines() {
            let line = line?;
            if let Ok(res) = serde_json::from_str::<EvalResult>(&line) {
                results.push(res);
            }
        }

        Ok(results)
    }
}
