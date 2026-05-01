use lsp_server::Connection;
use lsp_types::{
    CodeActionKind, CodeActionOptions, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkDoneProgressOptions, CodeLensOptions,
};
use std::error::Error;
use std::sync::{Arc, RwLock};

use scheme_toolbox_lsp::server::{Server, SharedState};
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

    let state = Arc::new(RwLock::new(SharedState {
        document_store: DocumentStore::new(),
    }));

    // Bounded channel to prevent OOM when user typing triggers many parses.
    // Stale tasks are handled via version checking or dropping.
    let (eval_tx, eval_rx) = crossbeam_channel::bounded(10);
    let (analysis_tx, analysis_rx) = crossbeam_channel::bounded(10);
    let (cancel_tx, cancel_rx) = crossbeam_channel::unbounded::<u32>();
    
    // Diagnostic worker for debouncing
    let (diag_tx, diag_rx) = crossbeam_channel::unbounded();
    let diag_lsp_sender = connection.sender.clone();
    let diag_worker_handle = std::thread::spawn(move || {
        diagnostic_worker(diag_rx, diag_lsp_sender);
    });

    // Spawn the eval worker. It owns the Evaluator (and thus the Racket REPL
    // child process) and is the only thread that ever calls into it.
    let worker_state = Arc::clone(&state);
    let worker_sender = DiagnosticWorkerSender {
        lsp_sender: connection.sender.clone(),
        diagnostic_tx: diag_tx.clone(),
    };
    let worker_handle = std::thread::spawn(move || {
        eval_worker(evaluator, eval_rx, cancel_rx, worker_state, worker_sender);
    });

    let analysis_worker_state = Arc::clone(&state);
    let analysis_worker_sender = DiagnosticWorkerSender {
        lsp_sender: connection.sender.clone(),
        diagnostic_tx: diag_tx,
    };
    let analysis_worker_handle = std::thread::spawn(move || {
        analysis_worker(analysis_evaluator, analysis_rx, analysis_worker_state, analysis_worker_sender);
    });

    let mut server = Server {
        eval_tx,
        analysis_tx,
        cancel_tx,
        state,
    };

    server.main_loop(&connection)?;

    eprintln!("LSP Main: loop finished, dropping server");
    drop(server);

    eprintln!("LSP Main: joining worker threads");
    worker_handle.join().map_err(|_| "Worker thread panicked")?;
    analysis_worker_handle.join().map_err(|_| "Analysis Worker thread panicked")?;
    diag_worker_handle.join().map_err(|_| "Diagnostic Worker thread panicked")?;

    // Explicitly drop connection to close the writer channel, allowing IO threads to exit.
    drop(connection);


    eprintln!("LSP Main: joining IO threads");
    io_threads.join()?;

    eprintln!("LSP Main: shutdown complete");
    Ok(())
    }