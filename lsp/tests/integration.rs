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
    let exec_cmd = r#"{"jsonrpc":"2.0","id":20,"method":"workspace/executeCommand","params":{"command":"scheme.evaluateSelection","arguments":["file:///sel.rkt","(+ 5 6)",{"start":{"line":2,"character":0},"end":{"line":2,"character":7}}]}}"#;
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

    let text = "#lang racket\n(+ 1 2)\n\n(+ 3 4)";
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


#[test]
fn test_notebook_diagnostic_downgrade() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. Evaluate a normal file with duplicate (should be ERROR)
    let eval_normal = r#"{"jsonrpc":"2.0","id":50,"method":"workspace/executeCommand","params":{"command":"scheme.evaluateSelection","arguments":["file:///normal.rkt","(define-values (x x) (values 1 2))",{"start":{"line":0,"character":0},"end":{"line":0,"character":33}}]}}"#;
    lsp.write_message(eval_normal);

    let mut found_error = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") && body.contains("normal.rkt") {
            // Diagnostic severity 1 is Error
            if body.contains("\"severity\":1") {
                found_error = true;
                break;
            }
        }
    }
    assert!(found_error, "Standard file should have ERROR severity for duplicate identifier");

    // 2. Evaluate a notebook cell with duplicate (should be WARNING)
    let eval_notebook = r#"{"jsonrpc":"2.0","id":51,"method":"workspace/executeCommand","params":{"command":"scheme.evaluateSelection","arguments":["vscode-notebook-cell:/test.rkt#cell1","(define-values (y y) (values 1 2))",{"start":{"line":0,"character":0},"end":{"line":0,"character":33}}]}}"#;
    lsp.write_message(eval_notebook);

    let mut found_warning = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") && body.contains("vscode-notebook-cell") {
            // Diagnostic severity 2 is Warning
            if body.contains("\"severity\":2") {
                found_warning = true;
                break;
            }
        }
    }
    assert!(found_warning, "Notebook cell should have WARNING severity for duplicate identifier");
}


#[test]
fn test_notebook_state_persistence() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. Define x in cell 1
    let eval_1 = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///state.rkt","code":"(define x 100)","executionId":1}}"#;
    lsp.write_message(eval_1);
    lsp.read_message_timeout(Duration::from_secs(5)); // Ack/Stream
    lsp.read_message_timeout(Duration::from_secs(5)); // Finished

    // 2. Use x in cell 2
    let eval_2 = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///state.rkt","code":"(+ x 50)","executionId":2}}"#;
    lsp.write_message(eval_2);
    
    let mut found_result = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("scheme/notebook/outputStream") && body.contains("150") {
            found_result = true;
            break;
        }
    }
    assert!(found_result, "Cell 2 could not access variable defined in Cell 1");

    // 3. Redefine x in cell 3 (should be allowed in REPL/Notebook mode)
    let eval_3 = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///state.rkt","code":"(define x 200) x","executionId":3}}"#;
    lsp.write_message(eval_3);

    let mut found_redefined = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("scheme/notebook/outputStream") && body.contains("200") {
            found_redefined = true;
            break;
        }
    }
    assert!(found_redefined, "Cell 3 could not redefine variable x");
}

#[test]
fn test_code_action() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let did_open = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///test.rkt","languageId":"racket","version":1,"text":"(+ 1 2)"}}}"#;
    lsp.write_message(did_open);

    let req = r#"{"jsonrpc":"2.0","id":100,"method":"textDocument/codeAction","params":{"textDocument":{"uri":"file:///test.rkt"},"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"context":{"diagnostics":[]}}}"#;
    lsp.write_message(req);

    let mut found_resp = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":100") {
            assert!(body.contains("scheme.evaluate"), "Expected evaluate command in code actions, got: {}", body);
            found_resp = true;
            break;
        }
    }
    assert!(found_resp, "Did not receive codeAction response");
}

#[test]
fn test_unknown_execute_command() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let req = r#"{"jsonrpc":"2.0","id":101,"method":"workspace/executeCommand","params":{"command":"unknown.command","arguments":[]}}"#;
    lsp.write_message(req);

    let mut found_resp = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":101") {
            assert!(body.contains("\"result\":null"), "Expected null result for unknown command, got: {}", body);
            found_resp = true;
            break;
        }
    }
    assert!(found_resp, "Did not receive response for unknown command");
}

#[test]
fn test_lang_fallback() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // Use a nonexistent language. read-syntax might still work if we use a mock module structure,
    // but the shim's read-syntax for #lang will likely fail if the reader is missing.
    // Instead, we'll test that even if the language require fails, we get some diagnostics.
    let text = "#lang nonexistent\n(+ 1 2)";
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///fallback.rkt","languageId":"racket","version":1,"text":"{}"}}}}}}"#, text.replace("\n", "\\n"));
    lsp.write_message(&did_open);

    let exec_cmd = r#"{"jsonrpc":"2.0","id":200,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///fallback.rkt"]}}"#;
    lsp.write_message(exec_cmd);

    let mut found_error = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") && body.contains("fallback.rkt") {
            if body.contains("\"is_error\":true") || body.contains("\"severity\":1") {
                found_error = true;
                break;
            }
        }
    }
    assert!(found_error, "Should have received error diagnostics for nonexistent language");
}

#[test]
fn test_evaluate_file_stdin() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Find the Racket binary. In tests, we can assume 'racket' is in PATH.
    // Or we can find our shim.
    let shim_path = std::env::current_dir().unwrap().join("src/eval-shim.rkt");
    
    let mut child = Command::new("racket")
        .arg(shim_path)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn racket");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    stdin.write_all(b"(+ 1 2)\n(display \"hello\")").expect("Failed to write to stdin");
    drop(stdin);

    let output = child.wait_with_output().expect("Failed to wait on child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    assert!(stdout.contains("\"result\":\"3\""), "Expected result 3, got: {}", stdout);
    assert!(stdout.contains("hello"), "Expected output 'hello', got: {}", stdout);
}

#[test]
fn test_document_lifecycle() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    let uri = "file:///lifecycle.rkt";
    
    // 1. didOpen
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{}","languageId":"racket","version":1,"text":"(+ 1 2)"}}}}}}"#, uri);
    lsp.write_message(&did_open);

    // Evaluate to get a hint
    let exec_cmd = format!(r#"{{"jsonrpc":"2.0","id":300,"method":"workspace/executeCommand","params":{{"command":"scheme.evaluate","arguments":["{}"]}}}}"#, uri);
    lsp.write_message(&exec_cmd);

    // Wait for diagnostics (evaluation finished)
    let mut found_diag = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("textDocument/publishDiagnostics") && body.contains("lifecycle.rkt") {
            found_diag = true;
            break;
        }
    }
    assert!(found_diag, "Did not receive diagnostics after open+eval");

    // Verify hint exists at character 7
    let hint_req = format!(r#"{{"jsonrpc":"2.0","id":301,"method":"textDocument/inlayHint","params":{{"textDocument":{{"uri":"{}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":1,"character":0}}}}}}}}"#, uri);
    lsp.write_message(&hint_req);
    
    let mut found_hint = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":301") {
            assert!(body.contains("\"character\":7"), "Expected hint at character 7, got: {}", body);
            found_hint = true;
            break;
        }
    }
    assert!(found_hint, "Did not receive inlay hint");

    // 2. didChange - Insert a newline at the start
    let did_change = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{}","version":2}},"contentChanges":[{{"text":"\n(+ 1 2)"}}]}}}}"#, uri);
    lsp.write_message(&did_change);

    // Hint should now be at line 1, character 7 due to shift_results
    let hint_req2 = format!(r#"{{"jsonrpc":"2.0","id":302,"method":"textDocument/inlayHint","params":{{"textDocument":{{"uri":"{}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":2,"character":0}}}}}}}}"#, uri);
    lsp.write_message(&hint_req2);

    let mut found_shifted_hint = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":302") {
            assert!(body.contains("\"line\":1"), "Expected hint shifted to line 1, got: {}", body);
            assert!(body.contains("\"character\":7"), "Expected hint still at character 7, got: {}", body);
            found_shifted_hint = true;
            break;
        }
    }
    assert!(found_shifted_hint, "Did not receive shifted inlay hint after didChange");

    // 3. didClose
    let did_close = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didClose","params":{{"textDocument":{{"uri":"{}"}}}}}}"#, uri);
    lsp.write_message(&did_close);

    // Request hints again - should be empty
    let hint_req3 = format!(r#"{{"jsonrpc":"2.0","id":303,"method":"textDocument/inlayHint","params":{{"textDocument":{{"uri":"{}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":2,"character":0}}}}}}}}"#, uri);
    lsp.write_message(&hint_req3);

    let mut found_empty_hints = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":303") {
            assert!(body.contains("\"result\":[]"), "Expected empty hints after didClose, got: {}", body);
            found_empty_hints = true;
            break;
        }
    }
    assert!(found_empty_hints, "Did not receive empty hints after didClose");
}

#[test]
fn test_notebook_concurrency() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // Send 3 evaluations rapidly
    for i in 0..3 {
        let eval_cell = format!(r#"{{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{{"uri":"file:///nb_conc.rkt","code":"(+ {} 10)","executionId":{}}}}}"#, i, i);
        lsp.write_message(&eval_cell);
    }

    let mut finished_count = 0;
    let mut results = Vec::new();

    for _ in 0..30 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("scheme/notebook/outputStream") && body.contains("\"type\":\"result\"") {
            results.push(body.clone());
        }
        
        if body.contains("scheme/notebook/evalFinished") {
            finished_count += 1;
        }
        
        if finished_count == 3 {
            break;
        }
    }

    assert_eq!(finished_count, 3, "Not all evaluations finished");
    assert_eq!(results.len(), 3, "Did not receive all results");
    
    // Verify values: 10, 11, 12
    let mut values: Vec<String> = results.iter()
        .map(|r| {
            let v: serde_json::Value = serde_json::from_str(r).unwrap();
            v["params"]["payload"]["data"].as_str().unwrap().to_string()
        })
        .collect();
    values.sort();
    assert_eq!(values, vec!["10", "11", "12"]);
}

#[test]
fn test_notebook_uri_isolation() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. Define x in Notebook A
    let eval_a = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///cell1.rkt","notebookUri":"file:///notebook_a.rktnb","code":"(define x 100)","executionId":1}}"#;
    lsp.write_message(eval_a);
    
    // Wait for finished
    for _ in 0..10 {
        let body = lsp.read_message_timeout(Duration::from_secs(5)).unwrap();
        if body.contains("scheme/notebook/evalFinished") && body.contains("\"executionId\":1") { break; }
    }

    // 2. Try to access x in Notebook B
    let eval_b = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///cell2.rkt","notebookUri":"file:///notebook_b.rktnb","code":"x","executionId":2}}"#;
    lsp.write_message(eval_b);

    let mut found_error = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("scheme/notebook/outputStream") && body.contains("\"type\":\"error\"") && body.contains("undefined") {
            found_error = true;
            break;
        }
    }
    assert!(found_error, "Notebook B should NOT be able to access variable from Notebook A");

    // 3. Access x in Notebook A again
    let eval_a2 = r#"{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{"uri":"file:///cell3.rkt","notebookUri":"file:///notebook_a.rktnb","code":"(+ x 5)","executionId":3}}"#;
    lsp.write_message(eval_a2);

    let mut found_result = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("scheme/notebook/outputStream") && body.contains("\"type\":\"result\"") && body.contains("105") {
            found_result = true;
            break;
        }
    }
    assert!(found_result, "Notebook A should be able to access its own variables");
}

#[test]
fn test_codelens_grouping_by_empty_lines() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. forms separated by single newline (group together)
    // 2. forms separated by double newline (separate groups)
    // 3. markdown block
    let text = "(define x 1)\n(define y 2)\n\n(define z 3)\n\n#| markdown\nhello\n|#\n\n(define w 4)";
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///grouping.rkt","languageId":"racket","version":1,"text":"{}"}}}}}}"#, text.replace("\n", "\\n"));
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
    let lens_req = r#"{"jsonrpc":"2.0","id":60,"method":"textDocument/codeLens","params":{"textDocument":{"uri":"file:///grouping.rkt"}}}"#;
    lsp.write_message(lens_req);

    let mut found_lenses = false;
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":60") {
            // Expected:
            // Group 1: (define x 1)\n(define y 2) -> 1 lens
            // Group 2: (define z 3) -> 1 lens
            // Group 3: markdown -> 0 executable lenses (but maybe 1 markdown range)
            // Group 4: (define w 4) -> 1 lens
            // Total: 3 executable lenses
            let lenses_count = body.matches("scheme.evaluateSelection").count();
            assert_eq!(lenses_count, 3, "Expected 3 code lenses, got: {}", body);
            found_lenses = true;
            break;
        }
    }
    assert!(found_lenses, "Did not receive code lens response");
}

#[test]
fn test_coordinate_drift_prevention() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. Open document (version 1) with an infinite loop to guarantee Racket stays busy
    let text_v1 = "(let loop () (loop))";
    let did_open = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///drift.rkt","languageId":"racket","version":1,"text":"{}"}}}}}}"#, text_v1.replace("\n", "\\n"));
    lsp.write_message(&did_open);

    // 2. Trigger evaluation for version 1
    let eval_req = r#"{"jsonrpc":"2.0","id":70,"method":"workspace/executeCommand","params":{"command":"scheme.evaluate","arguments":["file:///drift.rkt"]}}"#;
    lsp.write_message(eval_req);

    // Ensure the eval has time to start blocking
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 3. Send didChange to bump version to 2
    let text_v2 = "(define x 42)";
    let did_change = format!(r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"file:///drift.rkt","version":2}},"contentChanges":[^{{"text":"{}"}}^]}}}}"#, text_v2.replace("\n", "\\n")).replace("^", "[").replace("^", "]");
    lsp.write_message(&did_change);

    // 4. Send restartREPL to kill the infinite loop
    let restart_req = r#"{"jsonrpc":"2.0","id":71,"method":"workspace/executeCommand","params":{"command":"scheme.restartREPL","arguments":[]}}"#;
    lsp.write_message(restart_req);

    // 5. Verify that no diagnostics are published for version 1
    let mut found_v1_diag = false;

    // We expect the restart to happen and potentially other messages, but NEVER a v1 diagnostic
    for _ in 0..15 {
        let body = match lsp.read_message_timeout(Duration::from_secs(2)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("textDocument/publishDiagnostics") && body.contains("\"version\":1") {
            found_v1_diag = true;
        }
    }

    assert!(!found_v1_diag, "Should not publish diagnostics for outdated version 1");
}

#[test]
fn test_graceful_shutdown() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. Send shutdown request
    let shutdown_req = r#"{"jsonrpc":"2.0","id":500,"method":"shutdown","params":null}"#;
    lsp.write_message(shutdown_req);

    let mut found_resp = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":500") {
            assert!(body.contains("\"result\":null"), "Expected null result for shutdown, got: {}", body);
            found_resp = true;
            break;
        }
    }
    assert!(found_resp, "Did not receive shutdown response");

    // 2. Send exit notification
    let exit_not = r#"{"jsonrpc":"2.0","method":"exit","params":null}"#;
    lsp.write_message(exit_not);
    lsp.close_stdin();

    // Wait for process to exit
    let mut exited = false;
    let mut exit_status = None;
    for _ in 0..10 {
        if let Some(status) = lsp.child.try_wait().unwrap() {
            exited = true;
            exit_status = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(exited, "LSP process did not exit after exit notification");
    
    let status = exit_status.unwrap();
    assert!(status.success(), "LSP process did not exit cleanly (success/code 0). Got status: {}", status);
}

#[test]
fn test_pull_rich_media() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // 1. Evaluate a cell that produces a snip (rich media)
    let code = "(require racket/snip racket/class racket/draw) (make-object image-snip% (make-object bitmap% 1 1))";
    let eval_cell = format!(r#"{{"jsonrpc":"2.0","method":"scheme/notebook/evalCell","params":{{"uri":"file:///rich.rkt","code":"{}","executionId":60}}}}"#, code.replace("\"", "\\\""));
    lsp.write_message(&eval_cell);

    let mut rich_id = String::new();
    for _ in 0..20 {
        let body = match lsp.read_message_timeout(Duration::from_secs(10)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("scheme/notebook/outputStream") && body.contains("\"type\":\"rich\"") {
            // Extract the id
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(id) = v["params"]["payload"]["id"].as_str() {
                    rich_id = id.to_string();
                    break;
                }
            }
        }
    }
    assert!(!rich_id.is_empty(), "Did not receive rich media notification with ID");

    // 2. Pull the rich media
    let pull_req = format!(r#"{{"jsonrpc":"2.0","id":61,"method":"scheme/notebook/pullRichMedia","params":{{"id":"{}"}}}}"#, rich_id);
    lsp.write_message(&pull_req);

    let mut found_data = false;
    for _ in 0..10 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        if body.contains("\"id\":61") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(data) = v["result"]["data"].as_str() {
                    assert!(!data.is_empty(), "Received empty data for rich media pull");
                    found_data = true;
                    break;
                }
            }
        }
    }
    assert!(found_data, "Did not receive rich media data response");
}

