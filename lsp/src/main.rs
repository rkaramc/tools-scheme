use lsp_server::Connection;
use lsp_types::{
    CodeActionKind, CodeActionOptions, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkDoneProgressOptions, CodeLensOptions,
};
use std::error::Error;

use scheme_toolbox_lsp::server::Server;
use scheme_toolbox_lsp::worker::{eval_worker, analysis_worker, diagnostic_worker, DiagnosticWorkerSender};
use scheme_toolbox_lsp::documents::DocumentStore;
use scheme_toolbox_lsp::evaluator::Evaluator;

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        code_action_provider: Some(lsp_types::CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
            work_done_progress_options: WorkDoneProgressOptions::default(),
            resolve_provider: Some(false),
        })),
        inlay_hint_provider: Some(lsp_types::OneOf::Left(true)),
        code_lens_provider: Some(CodeLensOptions {
            resolve_provider: Some(false),
        }),
        execute_command_provider: Some(lsp_types::ExecuteCommandOptions {
            commands: vec![
                "scheme.evaluate".to_string(),
                "scheme.evaluateSelection".to_string(),
            ],
            ..Default::default()
        }),
        ..Default::default()
    })?;

    let initialization_params = connection.initialize(server_capabilities)?;
    let racket_path = initialization_params
        .get("initializationOptions")
        .and_then(|opts| opts.get("racketPath"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let evaluator = Evaluator::new(racket_path.clone())
        .map_err(|e| {
            eprintln!("LSP Initialization Error: {}", e);
            format!("Failed to initialize evaluator: {}", e)
        })?;
        
    let analysis_evaluator = Evaluator::new(racket_path)
        .map_err(|e| {
            eprintln!("LSP Initialization Error: {}", e);
            format!("Failed to initialize analysis evaluator: {}", e)
        })?;

    eprintln!("LSP Initialization: Racket Evaluator started successfully.");
    eprintln!("LSP Initialization: Racket binary: {}", evaluator.racket_path());
    eprintln!("LSP Initialization: Session log: {}", evaluator.session_path().display());

    let document_store = DocumentStore::new();

    // Unbounded channels to prevent deadlocks between Gateway and Workers.
    // Stale tasks are handled via version checking.
    let (eval_tx, eval_rx) = crossbeam_channel::unbounded();
    let (analysis_tx, analysis_rx) = crossbeam_channel::unbounded();
    let (cancel_tx, cancel_rx) = crossbeam_channel::unbounded::<u32>();
    
    // Result channel from workers back to the server
    let (result_tx, result_rx) = crossbeam_channel::unbounded();

    // Diagnostic worker for debouncing
    let (diag_tx, diag_rx) = crossbeam_channel::unbounded();
    let diag_lsp_sender = connection.sender.clone();
    let diag_worker_handle = std::thread::spawn(move || {
        diagnostic_worker(diag_rx, diag_lsp_sender);
    });

    // Spawn the eval worker. It owns the Evaluator (and thus the Racket REPL
    // child process) and is the only thread that ever calls into it.
    let worker_sender = DiagnosticWorkerSender {
        lsp_sender: connection.sender.clone(),
        diagnostic_tx: diag_tx.clone(),
    };
    let eval_result_tx = result_tx.clone();
    let worker_handle = std::thread::spawn(move || {
        eval_worker(evaluator, eval_rx, cancel_rx, eval_result_tx, worker_sender);
    });

    let analysis_worker_sender = DiagnosticWorkerSender {
        lsp_sender: connection.sender.clone(),
        diagnostic_tx: diag_tx.clone(),
    };
    let analysis_result_tx = result_tx;
    let analysis_worker_handle = std::thread::spawn(move || {
        analysis_worker(analysis_evaluator, analysis_rx, analysis_result_tx, analysis_worker_sender);
    });

    let mut server = Server {
        eval_tx,
        analysis_tx,
        cancel_tx,
        document_store,
        sender: DiagnosticWorkerSender {
            lsp_sender: connection.sender.clone(),
            diagnostic_tx: diag_tx.clone(),
        },
    };

    // Drop our local diag_tx so that diagnostic_worker rx will close when workers and server are gone.
    drop(diag_tx);

    server.main_loop(&connection, &result_rx)?;

    eprintln!("LSP Main: loop finished, dropping server");
    drop(server);

    eprintln!("LSP Main: joining worker threads");
    let _ = worker_handle.join();
    let _ = analysis_worker_handle.join();
    let _ = diag_worker_handle.join();

    // Explicitly drop connection to close the writer channel, allowing IO threads to exit.
    drop(connection);

    eprintln!("LSP Main: joining IO threads");
    io_threads.join()?;

    eprintln!("LSP Main: shutdown complete");
    Ok(())
    }