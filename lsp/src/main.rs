use lsp_server::Connection;
use lsp_types::{
    CodeActionKind, CodeActionOptions, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, WorkDoneProgressOptions, CodeLensOptions,
};
use std::error::Error;
use std::sync::{Arc, RwLock};

use scheme_toolbox_lsp::server::{Server, SharedState};
use scheme_toolbox_lsp::worker::{eval_worker, EvalTask};
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

    let evaluator = Evaluator::new(racket_path)
        .map_err(|e| {
            eprintln!("LSP Initialization Error: {}", e);
            format!("Failed to initialize evaluator: {}", e)
        })?;

    eprintln!("LSP Initialization: Racket Evaluator started successfully.");
    eprintln!("LSP Initialization: Racket binary: {}", evaluator.racket_path());
    eprintln!("LSP Initialization: Session log: {}", evaluator.session_path().display());

    let state = Arc::new(RwLock::new(SharedState {
        document_store: DocumentStore::new(),
    }));

    // Unbounded channel: dispatch is non-blocking. Stale Parse tasks are
    // skipped in the worker by checking the current document version.
    let (eval_tx, eval_rx) = crossbeam_channel::unbounded();

    // Spawn the eval worker. It owns the Evaluator (and thus the Racket REPL
    // child process) and is the only thread that ever calls into it.
    let worker_state = Arc::clone(&state);
    let worker_sender = connection.sender.clone();
    std::thread::spawn(move || {
        eval_worker(evaluator, eval_rx, worker_state, worker_sender);
    });

    let mut server = Server {
        eval_tx,
        state,
    };

    server.main_loop(&connection)?;
    io_threads.join()?;

    Ok(())
}