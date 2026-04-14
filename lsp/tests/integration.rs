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
    let mut found_result = false;
    for _ in 0..10 {
        let body = read_message(&mut reader);
        if body.contains("\"id\":2") {
            // The command response should contain the results as a list
            assert!(body.contains("\"result\":["));
            assert!(body.contains("\"result\":\"3\""));
            found_result = true;
            break;
        }
    }
    assert!(found_result, "Did not find executeCommand response for test.rkt");

    // 5. didOpen with #lang racket
    let lang_file = "{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{\"textDocument\":{\"uri\":\"file:///lang.rkt\",\"languageId\":\"racket\",\"version\":1,\"text\":\"#lang racket\\n(define y 100)\\n(+ y 20)\"}}}";
    write!(stdin, "Content-Length: {}\r\n\r\n{}", lang_file.len(), lang_file).unwrap();
    stdin.flush().unwrap();

    // 6. executeCommand for #lang file
    let exec_lang = r#"{"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///lang.rkt"]}}"#;
    write!(stdin, "Content-Length: {}\r\n\r\n{}", exec_lang.len(), exec_lang).unwrap();
    stdin.flush().unwrap();

    let mut found_lang_result = false;
    for _ in 0..10 {
        let body = read_message(&mut reader);
        if body.contains("\"id\":3") {
            assert!(body.contains("\"result\":\"120\""));
            found_lang_result = true;
            break;
        }
    }
    assert!(found_lang_result, "Did not find executeCommand response for #lang file");

    // 7. inlineValue request
    let inline_val_req = r#"{"jsonrpc":"2.0","id":4,"method":"textDocument/inlineValue","params":{"textDocument":{"uri":"file:///test.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":10,"character":0}},"context":{"frameId":0,"stoppedLocation":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}}}}}"#;
    write!(stdin, "Content-Length: {}\r\n\r\n{}", inline_val_req.len(), inline_val_req).unwrap();
    stdin.flush().unwrap();

    let mut found_inline_val = false;
    for _ in 0..10 {
        let body = read_message(&mut reader);
        if body.contains("\"id\":4") {
            assert!(body.contains("=> 3"));
            assert!(body.contains("=> void 📝"));
            found_inline_val = true;
            break;
        }
    }
    assert!(found_inline_val, "Did not find inlineValue response");

    child.kill().unwrap();
}

fn read_message<R: BufRead>(reader: &mut R) -> String {
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 {
            panic!("EOF reached while reading headers");
        }
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
