//! The application's routes, shared by the executable and integration tests.

use crate::http::borrowed_request::Request;
use crate::http::request::Method;
use crate::http::response::{Body, Response, Status};
use crate::http::uri::percent_decode;
use crate::router::Router;

pub fn default_router() -> Router {
    application_router().route_borrowed(Method::Get, "/", index)
}

/// Static mode serves the root index; exact application routes shadow files.
pub fn static_router(files: crate::static_files::StaticFiles) -> Router {
    application_router().fallback_borrowed(move |request| files.serve_borrowed(request))
}

fn application_router() -> Router {
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    Router::new()
        .route_borrowed(Method::Get, "/health", health)
        .route_borrowed(Method::Get, "/stream", |_| {
            Response::new(Status::Ok)
                .header("Content-Type", "text/event-stream; charset=utf-8")
                .header("Cache-Control", "no-cache")
                .header("X-Content-Type-Options", "nosniff")
                .body(Body::Stream {
                    interval: std::time::Duration::from_millis(250),
                    events: 10,
                })
        })
        .route_borrowed(Method::Post, "/echo", echo)
        .route_borrowed(Method::Get, "/headers", headers)
        .route_borrowed(Method::Get, "/hello", hello)
        .route_borrowed(Method::Get, "/counter", move |_request| {
            let count = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            Response::text(Status::Ok, &count.to_string())
        })
}

fn index(_request: &Request<'_>) -> Response {
    Response::text(Status::Ok, "hello\n")
}

fn health(_request: &Request<'_>) -> Response {
    Response::text(Status::Ok, "ok\n")
}

fn echo(request: &Request<'_>) -> Response {
    Response::new(Status::Ok)
        .header(
            "Content-Type",
            request
                .headers
                .get("Content-Type")
                .unwrap_or("application/octet-stream"),
        )
        .body(request.body.as_ref())
}

fn headers(request: &Request<'_>) -> Response {
    let mut text = String::new();
    for (name, value) in request.headers.iter() {
        text.push_str(name);
        text.push_str(": ");
        text.push_str(value);
        text.push('\n');
    }
    Response::text(Status::Ok, &text)
}

fn hello(request: &Request<'_>) -> Response {
    // Split the query before decoding so encoded '&' and '=' remain data.
    // The first name wins; every component is still validated, including duplicates.
    let mut name = None;
    for pair in request.query().unwrap_or("").split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let (key, value) = match (percent_decode(key), percent_decode(value)) {
            (Ok(key), Ok(value)) => (key, value),
            (Err(error), _) | (_, Err(error)) => {
                return Response::text(Status::BadRequest, &error.to_string());
            }
        };
        if key == "name" && name.is_none() {
            name = Some(value);
        }
    }
    Response::text(
        Status::Ok,
        &format!("hello, {}", name.as_deref().unwrap_or("world")),
    )
}
