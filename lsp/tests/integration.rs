use std::process::Command;
use std::path::PathBuf;
use std::io::{Write, BufRead, BufReader, Read};

#[test]
fn test_lsp_eval_integration() {
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
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit()) // See server logs in test output
        .spawn()
        .expect("failed to spawn LSP");

    let mut stdin = child.stdin.take().expect("no stdin");
    let stdout = child.stdout.take().expect("no stdout");
    let mut reader = BufReader::new(stdout);

    // 1. Initialize
    let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    write!(stdin, "Content-Length: {}\r\n\r\n{}", init_req.len(), init_req).unwrap();
    stdin.flush().unwrap();
    
    // Read init response
    let body = read_message(&mut reader);
    assert!(body.contains("capabilities"));

    // 2. Initialized notification
    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    write!(stdin, "Content-Length: {}\r\n\r\n{}", initialized.len(), initialized).unwrap();
    stdin.flush().unwrap();

    // 3. didOpen
    let did_open = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.rkt","languageId":"racket","version":1,"text":"(+ 1 2)\n(display \"hello\")"}}}"#;
    write!(stdin, "Content-Length: {}\r\n\r\n{}", did_open.len(), did_open).unwrap();
    stdin.flush().unwrap();

    // 4. executeCommand
    let exec_cmd = r#"{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///test.rkt"]}}"#;
    write!(stdin, "Content-Length: {}\r\n\r\n{}", exec_cmd.len(), exec_cmd).unwrap();
    stdin.flush().unwrap();

    // Read response for executeCommand
    // It might be a notification (diagnostics) or the response itself.
    // We'll read until we find the result.
    let mut found_result = false;
    for _ in 0..10 {
        let body = read_message(&mut reader);
        if body.contains("\"id\":2") {
            assert!(body.contains("\"result\":["));
            assert!(body.contains("\"result\":\"3\""));
            assert!(body.contains("\"output\":\"hello\""));
            found_result = true;
            break;
        }
    }
    assert!(found_result, "Did not find executeCommand response");

    child.kill().unwrap();
}

fn read_message<R: BufRead>(reader: &mut R) -> String {
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line.trim().is_empty() {
            break;
        }
        if line.starts_with("Content-Length:") {
            content_length = line.split(':').last().unwrap().trim().parse::<usize>().unwrap();
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).unwrap();
    String::from_utf8(body).unwrap()
}
