mod common;
use common::LspProcess;
use std::time::Duration;

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
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":2") && body.contains("\"result\"") {
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
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":3") && body.contains("\"result\"") {
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

#[test]
fn test_clear_namespace_removes_hints() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let did_open = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///clear.rkt","languageId":"racket","version":1,"text":"(+ 1 2)"}}}"#;
    lsp.write_message(did_open);

    let exec_cmd = r#"{"jsonrpc":"2.0","id":10,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///clear.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    // Wait for evaluation
    let mut found_diag = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") {
            found_diag = true;
            break;
        }
    }
    assert!(found_diag, "Did not receive publishDiagnostics");

    // Request inlay hints - should exist
    let hint_req = r#"{"jsonrpc":"2.0","id":11,"method":"textDocument/inlayHint","params":{"textDocument":{"uri":"file:///clear.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":0}}}}"#;
    lsp.write_message(hint_req);
    
    let mut found_hints = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        println!("MSG id:11 loop: {}", body);
        if body.contains("\"id\":11") {
            assert!(body.contains("\"result\":["), "Expected non-empty result for hints, got: {}", body);
            found_hints = true;
            break;
        }
    }
    assert!(found_hints, "Did not receive inlay hints");

    // Clear namespace
    let clear_cmd = r#"{"jsonrpc":"2.0","id":12,"method":"workspace/executeCommand","params":{"command":"scheme.clearNamespace","arguments":["file:///clear.rkt"]}}"#;
    lsp.write_message(clear_cmd);
    
    let mut found_clear_ack = false;
    let mut found_refresh = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        println!("MSG id:12 loop: {}", body);
        if body.contains("\"id\":12") {
            found_clear_ack = true;
        }
        if body.contains("workspace/inlayHint/refresh") {
            found_refresh = true;
        }
        if found_clear_ack && found_refresh {
            break;
        }
    }
    assert!(found_clear_ack, "Did not receive clear ack");
    assert!(found_refresh, "Did not receive inlay hint refresh");

    // Request inlay hints again - should be empty!
    let hint_req2 = r#"{"jsonrpc":"2.0","id":13,"method":"textDocument/inlayHint","params":{"textDocument":{"uri":"file:///clear.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":1,"character":0}}}}"#;
    lsp.write_message(hint_req2);
    
    let mut found_empty_hints = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        println!("MSG id:13 loop: {}", body);
        if body.contains("\"id\":13") {
            assert!(body.contains("\"result\":[]"), "Expected empty hints after clear, got: {}", body);
            found_empty_hints = true;
            break;
        }
    }
    assert!(found_empty_hints, "Did not receive empty inlay hints after clear");
}

#[test]
fn test_evaluate_selection_offset() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let text = "(+ 1 2)\n(+ 3 4)\n(+ 5 6)";
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///sel.rkt","languageId":"racket","version":1,"text":"{}"}}}}}}"#, text.replace("\n", "\\n"));
    lsp.write_message(&did_open);

    // Evaluate the third line, passing the offset {line: 2, character: 0}
    let exec_cmd = r#"{"jsonrpc":"2.0","id":20,"method":"workspace/executeCommand","params":{"command":"scheme.evaluateSelection","arguments":["file:///sel.rkt","(+ 5 6)",{"line":2,"character":0}]}}"#;
    lsp.write_message(exec_cmd);

    // Wait for evaluation
    let mut found_diag = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") {
            found_diag = true;
            break;
        }
    }
    assert!(found_diag, "Did not receive publishDiagnostics");

    // Request inlay hints
    let hint_req = r#"{"jsonrpc":"2.0","id":21,"method":"textDocument/inlayHint","params":{"textDocument":{"uri":"file:///sel.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":3,"character":0}}}}"#;
    lsp.write_message(hint_req);
    
    let mut found_hints = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        println!("MSG: {}", body);
        if body.contains("\"id\":21") {
            // The position should be on line 2, character 7
            assert!(body.contains("\"line\":2"), "Expected hint on line 2, got: {}", body);
            assert!(body.contains("\"character\":7"), "Expected hint at character 7, got: {}", body);
            found_hints = true;
            break;
        }
    }
    assert!(found_hints, "Did not receive inlay hints");
}

#[test]
fn test_lang_file_code_lenses() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let text = "#lang racket\n(+ 1 2)\n(+ 3 4)";
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///lang-lens.rkt","languageId":"racket","version":1,"text":"{}"}}}}}}"#, text.replace("\n", "\\n"));
    lsp.write_message(&did_open);

    // Wait for the background parse task to finish
    let mut found_refresh = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("workspace/codeLens/refresh") {
            found_refresh = true;
            break;
        }
    }
    assert!(found_refresh, "Did not receive code lens refresh after open");

    // Request code lenses
    let lens_req = r#"{"jsonrpc":"2.0","id":30,"method":"textDocument/codeLens","params":{"textDocument":{"uri":"file:///lang-lens.rkt"}}}"#;
    lsp.write_message(lens_req);

    let mut found_lenses = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":30") {
            // We should get 2 code lenses because there are 2 forms (+ 1 2) and (+ 3 4)
            // (The #lang racket line itself is part of the module declaration)
            let lenses_count = body.matches("scheme.evaluateSelection").count();
            assert_eq!(lenses_count, 2, "Expected 2 code lenses, got: {}", body);
            found_lenses = true;
            break;
        }
    }
    assert!(found_lenses, "Did not receive code lens response");

    // Evaluate the whole file to ensure it doesn't throw 'racket: undefined'
    let exec_cmd = r#"{"jsonrpc":"2.0","id":31,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///lang-lens.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    let mut found_diag = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") {
            assert!(!body.contains("racket: undefined"), "Received 'racket: undefined' error!");
            assert!(!body.contains("\"is_error\":true"), "Received evaluation error: {}", body);
            found_diag = true;
            break;
        }
    }
    assert!(found_diag, "Did not receive publishDiagnostics");
}

#[test]
fn test_clear_namespace_preserves_codelens() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let text = "(+ 1 2)\n(+ 3 4)";
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///clear-lens.rkt","languageId":"racket","version":1,"text":"{}"}}}}}}"#, text.replace("\n", "\\n"));
    lsp.write_message(&did_open);

    let mut found_refresh = false;
    for _ in 0..15 {
        if let Some(body) = lsp.read_message_timeout(std::time::Duration::from_secs(10)) {
            if body.contains("workspace/codeLens/refresh") {
                found_refresh = true;
                break;
            }
        }
    }
    assert!(found_refresh, "Did not receive code lens refresh after open");

    // Clear namespace
    let clear_cmd = r#"{"jsonrpc":"2.0","id":12,"method":"workspace/executeCommand","params":{"command":"scheme.clearNamespace","arguments":["file:///clear-lens.rkt"]}}"#;
    lsp.write_message(clear_cmd);
    
    let mut found_clear_ack = false;
    let mut found_second_refresh = false;
    for _ in 0..20 {
        if let Some(body) = lsp.read_message_timeout(std::time::Duration::from_secs(10)) {
            if body.contains("\"id\":12") {
                found_clear_ack = true;
            }
            if body.contains("workspace/codeLens/refresh") {
                found_second_refresh = true;
            }
            if found_clear_ack && found_second_refresh {
                break;
            }
        }
    }
    assert!(found_clear_ack, "Did not receive clear ack");
    assert!(found_second_refresh, "Did not receive second code lens refresh");

    // Request code lenses - should NOT be empty
    let req = r#"{"jsonrpc":"2.0","id":20,"method":"textDocument/codeLens","params":{"textDocument":{"uri":"file:///clear-lens.rkt"}}}"#;
    lsp.write_message(req);

    let mut found_lenses = false;
    for _ in 0..15 {
        if let Some(body) = lsp.read_message_timeout(std::time::Duration::from_secs(10)) {
            if body.contains("\"id\":20") {
                assert!(body.contains("scheme.evaluateSelection"), "Code lenses missing: {}", body);
                found_lenses = true;
                break;
            }
        }
    }
    assert!(found_lenses, "Did not receive code lens response");
}

#[test]
fn test_notebook_eval() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let eval_cell = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///notebook.rkt","code":"(display \"hello notebook\\n\") (+ 1 2)","executionId":42}}"#;
    lsp.write_message(eval_cell);

    let mut found_stdout = false;
    let mut found_result = false;
    let mut found_finished = false;

    for _ in 0..20 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("scheme/notebook/outputStream") {
            if body.contains("hello notebook") && body.contains("\"type\":\"stdout\"") {
                found_stdout = true;
            }
            if body.contains("3") && body.contains("\"type\":\"result\"") {
                found_result = true;
            }
        }
        
        if body.contains("scheme/notebook/evalFinished") && body.contains("\"executionId\":42") {
            found_finished = true;
        }
        
        if found_stdout && found_result && found_finished {
            break;
        }
    }

    assert!(found_stdout, "Did not receive stdout output stream");
    assert!(found_result, "Did not receive result output stream");
    assert!(found_finished, "Did not receive evalFinished notification");
}


#[test]
fn test_notebook_cancel_eval() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // Send an infinite loop
    let eval_cell = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///notebook_cancel.rkt","code":"(let loop () (loop))","executionId":43}}"#;
    lsp.write_message(eval_cell);

    // Wait a brief moment to ensure it started
    std::thread::sleep(Duration::from_millis(500));

    // Send cancellation
    let cancel_eval = r#"{"jsonrpc":"2.0","method":"scheme/notebook/cancelEval","params":{"uri":"file:///notebook_cancel.rkt","executionId":43}}"#;
    lsp.write_message(cancel_eval);

    let mut found_finished = false;
    let mut found_error = false;

    for _ in 0..20 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("scheme/notebook/outputStream") && body.contains("\"type\":\"error\"") && body.contains("cancelled") {
            found_error = true;
        }
        
        if body.contains("scheme/notebook/evalFinished") && body.contains("\"executionId\":43") && body.contains("\"success\":true") {
            found_finished = true;
        }
        
        if found_error && found_finished {
            break;
        }
    }

    assert!(found_error, "Did not receive error output stream for cancellation");
    assert!(found_finished, "Did not receive evalFinished notification with success=true");

    // The evaluator should still be alive, so a subsequent evaluation should work immediately
    let eval_cell2 = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///notebook_cancel.rkt","code":"(+ 10 20)","executionId":44}}"#;
    lsp.write_message(eval_cell2);

    let mut found_result = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("scheme/notebook/outputStream") && body.contains("30") {
            found_result = true;
            break;
        }
    }
    assert!(found_result, "Evaluator died! Subsequent evaluation failed.");
}

