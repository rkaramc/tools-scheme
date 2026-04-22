use lsp_server::{Request, RequestId, Response, Notification};
use lsp_types::request::{CodeActionRequest, ExecuteCommand};
use lsp_types::notification::DidOpenTextDocument;

pub fn check_req(req: Request) -> Result<(), Box<dyn std::error::Error>> {
    let req = match lsp_server::RequestDispatcher::new(req)
        .on_sync_mut::<CodeActionRequest>(|id, params| Ok(()))?
        .finish() {
            Ok(_) => return Ok(()),
            Err(req) => req,
        };
    Ok(())
}

pub fn check_not(not: Notification) -> Result<(), Box<dyn std::error::Error>> {
    let not = match lsp_server::NotificationDispatcher::new(not)
        .on_sync_mut::<DidOpenTextDocument>(|params| Ok(()))?
        .finish() {
            Ok(_) => return Ok(()),
            Err(not) => not,
        };
    Ok(())
}
