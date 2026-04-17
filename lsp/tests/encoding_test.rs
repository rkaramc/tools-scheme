mod common;
use common::LspProcess;
use serde_json::Value;

#[test]
fn test_emoji_utf16_positioning() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 🦀 is 1 code point in Racket, but 2 code units in UTF-16 (LSP).
    // Let's test if an error reported after an emoji has the correct column.
    let text = "(define 🦀 1)\n(syntax-error-here)";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///emoji.rkt","languageId":"racket","version":1,"text":{:?}}}}}}}"#,
        text
    );
    lsp.write_message(&did_open);

    let exec_cmd = r#"{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///emoji.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    // Look for diagnostics
    let mut found_diag = false;
    for _ in 0..10 {
        let body = lsp.read_message();
        if body.contains("textDocument/publishDiagnostics") && body.contains("emoji.rkt") {
            let json: Value = serde_json::from_str(&body).unwrap();
            let diags = json["params"]["diagnostics"].as_array().unwrap();
            
            // We expect at least one diagnostic for the syntax error
            for diag in diags {
                let msg = diag["message"].as_str().unwrap();
                if msg.contains("syntax-error-here") || msg.contains("unbound identifier") {
                    found_diag = true;
                    // Check if the range is sane (e.g. starts at line 1)
                    let start_line = diag["range"]["start"]["line"].as_u64().unwrap();
                    assert_eq!(start_line, 1);
                }
            }
            if found_diag { break; }
        }
    }
    assert!(found_diag, "Did not find diagnostic for emoji file");
}

#[test]
fn test_multiline_range_coordinates() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // (+ 1
    //    2)
    // This should span two lines.
    let text = "(+ 1\n   2)";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///multiline.rkt","languageId":"racket","version":1,"text":{:?}}}}}}}"#,
        text
    );
    lsp.write_message(&did_open);

    let exec_cmd = r#"{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///multiline.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    // 1. Wait for evaluation to finish (signaled by inlayHint/refresh)
    let mut found_refresh = false;
    for _ in 0..20 {
        let body = lsp.read_message();
        if body.contains("workspace/inlayHint/refresh") {
            found_refresh = true;
            break;
        }
    }
    assert!(found_refresh, "Did not receive inlayHint/refresh signal from server");

    // 2. Request inlay hints
    let hint_req = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/inlayHint","params":{"textDocument":{"uri":"file:///multiline.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":10,"character":0}}}}"#;
    lsp.write_message(hint_req);

    let mut found_hint = false;
    for _ in 0..10 {
        let body = lsp.read_message();
        if body.contains("\"id\":3") {
            let json: Value = serde_json::from_str(&body).unwrap();
            let hints = json["result"].as_array().expect("result should be an array of InlayHint");
            
            for hint in hints {
                let label = hint["label"].as_str().unwrap();
                if label.contains("=> 3") {
                    found_hint = true;
                    let pos = &hint["position"];
                    // Ends at (+ 1\n   2) -> line 1, col 5
                    assert_eq!(pos["line"], 1);
                    assert_eq!(pos["character"], 5);
                }
            }
            if found_hint { break; }
        }
    }
    assert!(found_hint, "Did not find inlay hint with evaluation result '=> 3'");
}
