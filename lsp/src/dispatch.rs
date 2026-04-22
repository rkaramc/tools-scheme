use std::error::Error;
use lsp_server::{Request, RequestId, Notification};
use serde::de::DeserializeOwned;

pub struct RequestDispatcher {
    pub req: Option<Request>,
}

impl RequestDispatcher {
    pub fn new(req: Request) -> Self {
        RequestDispatcher { req: Some(req) }
    }

    pub fn on_sync_mut<R>(
        &mut self,
        f: impl FnOnce(RequestId, R::Params) -> Result<(), Box<dyn Error + Sync + Send>>,
    ) -> Result<&mut Self, Box<dyn Error + Sync + Send>>
    where
        R: lsp_types::request::Request,
        R::Params: DeserializeOwned,
    {
        let (id, params) = match self.parse::<R>() {
            Some(it) => it,
            None => return Ok(self),
        };
        f(id, params)?;
        self.req = None;
        Ok(self)
    }

    pub fn finish(&mut self) -> Option<Request> {
        self.req.take()
    }

    fn parse<R>(&mut self) -> Option<(RequestId, R::Params)>
    where
        R: lsp_types::request::Request,
        R::Params: DeserializeOwned,
    {
        let req = self.req.as_ref()?;
        if req.method != R::METHOD {
            return None;
        }
        let req = self.req.take().unwrap();
        let params = serde_json::from_value::<R::Params>(req.params).ok()?;
        Some((req.id, params))
    }
}

pub struct NotificationDispatcher {
    pub not: Option<Notification>,
}

impl NotificationDispatcher {
    pub fn new(not: Notification) -> Self {
        NotificationDispatcher { not: Some(not) }
    }

    pub fn on_sync_mut<N>(
        &mut self,
        f: impl FnOnce(N::Params) -> Result<(), Box<dyn Error + Sync + Send>>,
    ) -> Result<&mut Self, Box<dyn Error + Sync + Send>>
    where
        N: lsp_types::notification::Notification,
        N::Params: DeserializeOwned,
    {
        let params = match self.parse::<N>() {
            Some(it) => it,
            None => return Ok(self),
        };
        f(params)?;
        self.not = None;
        Ok(self)
    }

    pub fn finish(&mut self) -> Option<Notification> {
        self.not.take()
    }

    fn parse<N>(&mut self) -> Option<N::Params>
    where
        N: lsp_types::notification::Notification,
        N::Params: DeserializeOwned,
    {
        let not = self.not.as_ref()?;
        if not.method != N::METHOD {
            return None;
        }
        let not = self.not.take().unwrap();
        let params = serde_json::from_value::<N::Params>(not.params).ok()?;
        Some(params)
    }
}
