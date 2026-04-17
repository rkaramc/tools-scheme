mod common;
use common::LspProcess;

#[test]
fn test_lsp_eval_integration() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. didOpen
    let did_open = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.rkt","languageId":"racket","version":1,"text":"(+ 1 2)\n(display \"hello\")"}}}"#;
    lsp.write_message(did_open);

    // 2. executeCommand — evaluation is async.
    let exec_cmd = r#"{"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///test.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    // Collect messages looking for ack and diagnostic
    let mut found_ack = false;
    let mut found_diag = false;
    for _ in 0..15 {
        let body = lsp.read_message();
        if body.contains("\"id\":2") {
            assert!(body.contains("\"result\":null"), "Expected null ack, got: {}", body);
            found_ack = true;
        }
        if body.contains("textDocument/publishDiagnostics") {
            found_diag = true;
        }
        if found_ack && found_diag {
            break;
        }
    }
    assert!(found_ack, "Did not receive null ack for executeCommand id:2");
    assert!(found_diag, "Did not receive publishDiagnostics notification after evaluation");

    // 3. didOpen with #lang racket
    let lang_file = r##"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///lang.rkt","languageId":"racket","version":1,"text":"#lang racket\n(define y 100)\n(+ y 20)"}}}"##;
    lsp.write_message(lang_file);

    // 4. executeCommand for #lang file
    let exec_lang = r#"{"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///lang.rkt"]}}"#;
    lsp.write_message(exec_lang);

    let mut found_lang_ack = false;
    let mut found_lang_diag = false;
    for _ in 0..15 {
        let body = lsp.read_message();
        if body.contains("\"id\":3") {
            assert!(body.contains("\"result\":null"), "Expected null ack, got: {}", body);
            found_lang_ack = true;
        }
        if body.contains("textDocument/publishDiagnostics") && body.contains("lang.rkt") {
            found_lang_diag = true;
        }
        if found_lang_ack && found_lang_diag {
            break;
        }
    }
    assert!(found_lang_ack, "Did not receive null ack for executeCommand id:3");
    assert!(found_lang_diag, "Did not receive publishDiagnostics for #lang file");
}
