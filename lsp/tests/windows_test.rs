mod common;
use common::LspProcess;
use serde_json::Value;
use std::time::Duration;

#[test]
fn test_crlf_drift_stress() {
    // Generate a file with 50 lines of CRLF content
    let mut text = String::new();
    for i in 1..=50 {
        text.push_str(&format!("(+ {} {})\r\n", i, i));
    }

    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///drift.rkt","languageId":"racket","version":1,"text":{:?}}}}}}}"#,
        text
    );
    lsp.write_message(&did_open);

    let exec_cmd = r#"{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///drift.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    // 1. Wait for evaluate signal (inlayHint/refresh)
    let mut found_refresh = false;
    for _ in 0..100 { 
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        // println!("Received: {}", body);
        if body.contains("workspace/inlayHint/refresh") {
            found_refresh = true;
            break;
        }
    }
    assert!(found_refresh, "Did not receive inlayHint/refresh signal from server");

    // 2. Request inlay hints for the whole file
    let hint_req = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/inlayHint","params":{"textDocument":{"uri":"file:///drift.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":100,"character":0}}}}"#;
    lsp.write_message(hint_req);

    let mut last_line_correct = false;
    let mut results_count = 0;

    let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
        Some(b) => b,
        None => panic!("Timeout waiting for inlay hint response"),
    };
    // println!("Received in loop 2: {}", body);
    if body.contains("\"id\":3") {
        let json: Value = serde_json::from_str(&body).unwrap();
        let hints = json["result"].as_array().expect("result should be an array of InlayHint");
        results_count = hints.len();

        for hint in hints {
            if hint["position"]["line"] == 49 {
                let label = hint["label"].as_str().unwrap();
                if label.contains("=> 100") {
                    last_line_correct = true;
                }
            }
        }
    }

    assert!(results_count >= 50, "Expected at least 50 inlay hints, found: {}", results_count);
    assert!(last_line_correct, "The 50th evaluation result (line 49) was not found or had incorrect value/position.");
}
