use std::process::{Command, Child, Stdio};
use std::path::PathBuf;
use std::io::{Read, Write, BufRead, BufReader};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
use std::thread;

pub struct LspProcess {
    pub child: Child,
    pub stdin: Option<std::process::ChildStdin>,
    pub rx: Receiver<String>,
}

impl LspProcess {
    pub fn spawn() -> Self {
        let mut lsp_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        lsp_path.pop();
        lsp_path.push("target");
        lsp_path.push("debug");
        let bin_name = if cfg!(windows) { "scheme-toolbox-lsp.exe" } else { "scheme-toolbox-lsp" };
        lsp_path.push(bin_name);

        let mut shim_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        shim_path.push("src");
        shim_path.push("eval-shim.rkt");

        let mut child = Command::new(&lsp_path)
            .arg(&shim_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn LSP");

        let stdin = child.stdin.take().expect("no stdin");
        let stdout = child.stdout.take().expect("no stdout");
        let stderr = child.stderr.take().expect("no stderr");
        
        let (tx, rx) = mpsc::channel();
        
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    break;
                }
                if line.starts_with("Content-Length:") {
                    let content_length: usize = line.split(':').last().unwrap().trim().parse().unwrap();
                    // consume empty line
                    reader.read_line(&mut line).unwrap();
                    let mut body = vec![0u8; content_length];
                    if reader.read_exact(&mut body).is_ok() {
                        if let Ok(msg) = String::from_utf8(body) {
                            if tx.send(msg).is_err() {
                                break;
                            }
                        }
                    }
                }
            }
        });

        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                if let Ok(l) = line {
                    eprintln!("LSP STDERR: {}", l);
                }
            }
        });

        Self { child, stdin: Some(stdin), rx }
    }

    pub fn write_message(&mut self, msg: &str) {
        if let Some(ref mut stdin) = self.stdin {
            write!(stdin, "Content-Length: {}\r\n\r\n{}", msg.len(), msg).unwrap();
            stdin.flush().unwrap();
        }
    }

    pub fn close_stdin(&mut self) {
        self.stdin.take();
    }

    pub fn read_message(&mut self) -> String {
        self.rx.recv().expect("failed to read message")
    }

    pub fn read_message_timeout(&mut self, timeout: Duration) -> Option<String> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn initialize(&mut self) {
        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
        self.write_message(init_req);
        let body = self.read_message();
        assert!(body.contains("capabilities"));

        let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
        self.write_message(initialized);
    }
}

impl Drop for LspProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}
