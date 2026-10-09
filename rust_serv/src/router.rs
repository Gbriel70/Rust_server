//! Exact, case-sensitive routes. Path decoding happens one segment at a time.

use std::collections::HashMap;

use crate::http::request::{Method, Request};
use crate::http::response::{Response, Status};
use crate::http::uri::decode_path;

type Handler = Box<dyn Fn(&Request) -> Response + Send + Sync>;

#[derive(Default)]
pub struct Router {
    routes: HashMap<String, HashMap<Method, Handler>>,
    fallback: Option<Handler>,
}

impl Router {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a decoded path. Registering the same method/path replaces its handler.
    /// HEAD is generated from GET and cannot be registered separately in this router.
    pub fn route(
        mut self,
        method: Method,
        path: &str,
        handler: impl Fn(&Request) -> Response + Send + Sync + 'static,
    ) -> Self {
        assert_ne!(
            method,
            Method::Head,
            "register GET to provide HEAD automatically"
        );
        self.routes
            .entry(path.to_owned())
            .or_default()
            .insert(method, Box::new(handler));
        self
    }

    /// Exact registered paths take precedence, including their method restrictions.
    pub fn fallback(
        mut self,
        handler: impl Fn(&Request) -> Response + Send + Sync + 'static,
    ) -> Self {
        self.fallback = Some(Box::new(handler));
        self
    }

    pub fn handle(&self, request: &Request) -> Response {
        let path = match decode_path(request.path()) {
            Ok(path) => path,
            Err(error) => return Response::text(Status::BadRequest, &error.to_string()),
        };
        let Some(methods) = self.routes.get(&path) else {
            return self.fallback.as_ref().map_or_else(
                || Response::text(Status::NotFound, "not found\n"),
                |handler| handler(request),
            );
        };
        // HEAD is derived from GET; the server suppresses the body on the wire.
        let method = if request.method == Method::Head {
            Method::Get
        } else {
            request.method
        };
        if let Some(handler) = methods.get(&method) {
            return handler(request);
        }
        let mut allowed: Vec<_> = methods.keys().map(|method| method.as_str()).collect();
        if methods.contains_key(&Method::Get) {
            allowed.push(Method::Head.as_str());
        }
        allowed.sort_unstable();
        allowed.dedup();
        Response::text(Status::MethodNotAllowed, "method not allowed\n")
            .header("Allow", allowed.join(", "))
    }
}
