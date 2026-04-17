use std::process::{Command, Child, Stdio};
use std::path::PathBuf;
use std::io::{Read, Write, BufRead, BufReader};

pub struct LspProcess {
    pub child: Child,
    pub stdin: std::process::ChildStdin,
    pub reader: BufReader<std::process::ChildStdout>,
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
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn LSP");

        let stdin = child.stdin.take().expect("no stdin");
        let stdout = child.stdout.take().expect("no stdout");
        let reader = BufReader::new(stdout);

        Self { child, stdin, reader }
    }

    pub fn write_message(&mut self, msg: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", msg.len(), msg).unwrap();
        self.stdin.flush().unwrap();
    }

    pub fn read_message(&mut self) -> String {
        let mut content_length = 0;
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if line.starts_with("Content-Length:") {
                content_length = line.split(':').last().unwrap().trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0u8; content_length];
        self.reader.read_exact(&mut body).unwrap();
        String::from_utf8(body).unwrap()
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
