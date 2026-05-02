mod common;
use common::LspProcess;
use std::time::Duration;

#[test]
fn test_streaming_output_truncation() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // MAX-OUTPUT-SIZE is 10000 by default.
    // We'll print 15000 characters.
    let code = "(display (make-string 15000 #\\A))";
    let params = serde_json::json!({
        "uri": "file:///trunc.rkt",
        "code": code,
        "executionId": 100
    });
    let eval_cell = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "scheme/notebook/evalCell",
        "params": params
    }).to_string();
    lsp.write_message(&eval_cell);

    let mut total_output_len = 0;
    let mut found_truncation = false;

    for _ in 0..100 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("scheme/notebook/outputStream") {
            if body.contains("\"type\":\"stdout\"") {
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(payload) = json_val.get("params").and_then(|p| p.get("payload")) {
                        if let Some(data) = payload.get("data").and_then(|d| d.as_str()) {
                            total_output_len += data.replace("... [truncated]", "").len();
                            if data.contains("[truncated]") {
                                found_truncation = true;
                            }
                        }
                    }
                }
            }
        }
        
        if body.contains("scheme/notebook/evalFinished") && body.contains("\"executionId\":100") {
            break;
        }
    }

    // It should be around 10000. Allow some buffer for JSON overhead but we are checking data len.
    assert!(total_output_len <= 10005, "Output length exceeded limit: {}", total_output_len);
    assert!(found_truncation, "Did not find truncation message in output");

    // Second evaluation - should also have its own 10000 limit, resetting from previous eval
    let eval_cell_2 = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "scheme/notebook/evalCell",
        "params": {
            "uri": "file:///trunc.rkt",
            "code": "(display (make-string 5000 #\\B))",
            "executionId": 101
        }
    }).to_string();
    lsp.write_message(&eval_cell_2);

    let mut total_output_len_2 = 0;
    let mut found_truncation_2 = false;

    for _ in 0..100 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("scheme/notebook/outputStream") {
            if body.contains("\"type\":\"stdout\"") && (body.contains("B") || body.contains("66")) {
                if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(payload) = json_val.get("params").and_then(|p| p.get("payload")) {
                        if let Some(data) = payload.get("data").and_then(|d| d.as_str()) {
                            // Racket might output bytes as escapes if not careful, but make-string 5000 #"B" should be fine.
                            total_output_len_2 += data.replace("... [truncated]", "").len();
                            if data.contains("[truncated]") {
                                found_truncation_2 = true;
                            }
                        }
                    }
                }
            }
        }
        
        if body.contains("scheme/notebook/evalFinished") && body.contains("\"executionId\":101") {
            break;
        }
    }

    assert_eq!(total_output_len_2, 5000, "Second evaluation output length was NOT 5000: {}", total_output_len_2);
    assert!(!found_truncation_2, "Second evaluation should NOT have been truncated");
}

#[test]
fn test_streaming_output_truncation_independent_streams() {
    let mut lsp = LspProcess::spawn();
    lsp.initialize();

    // Print 8000 to stdout, 8000 to stderr, then another 8000 to each.
    // Each should be truncated at 10000.
    let code = "(begin (display (make-string 8000 #\\A)) (eprintf (make-string 8000 #\\E)) (display (make-string 8000 #\\A)) (eprintf (make-string 8000 #\\E)))";
    let params = serde_json::json!({
        "uri": "file:///trunc_independent.rkt",
        "code": code,
        "executionId": 200
    });
    let eval_cell = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "scheme/notebook/evalCell",
        "params": params
    }).to_string();
    lsp.write_message(&eval_cell);

    let mut stdout_len = 0;
    let mut stderr_len = 0;
    let mut stdout_truncated = false;
    let mut stderr_truncated = false;

    for _ in 0..100 {
        let body = match lsp.read_message_timeout(Duration::from_secs(5)) {
            Some(b) => b,
            None => break,
        };
        
        if body.contains("scheme/notebook/outputStream") {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(payload) = json_val.get("params").and_then(|p| p.get("payload")) {
                    let stream_type = payload.get("type").and_then(|t| t.as_str());
                    if let Some(data) = payload.get("data").and_then(|d| d.as_str()) {
                        let clean_data = data.replace("... [truncated]", "");
                        if stream_type == Some("stdout") {
                            stdout_len += clean_data.len();
                            if data.contains("[truncated]") { stdout_truncated = true; }
                        } else if stream_type == Some("stderr") {
                            stderr_len += clean_data.len();
                            if data.contains("[truncated]") { stderr_truncated = true; }
                        }
                    }
                }
            }
        }
        
        if body.contains("scheme/notebook/evalFinished") && body.contains("\"executionId\":200") {
            break;
        }
    }

    assert!(stdout_len <= 10005, "Stdout length exceeded limit: {}", stdout_len);
    assert!(stdout_len >= 9990, "Stdout length too short: {}", stdout_len);
    assert!(stdout_truncated, "Stdout was NOT truncated");

    assert!(stderr_len <= 10005, "Stderr length exceeded limit: {}", stderr_len);
    assert!(stderr_len >= 9990, "Stderr length too short: {}", stderr_len);
    assert!(stderr_truncated, "Stderr was NOT truncated");
}
