//! The application's routes, shared by the executable and integration tests.

use crate::http::request::{Method, Request};
use crate::http::response::{Response, Status};
use crate::http::uri::percent_decode;
use crate::router::Router;

pub fn default_router() -> Router {
    let counter = std::cell::Cell::new(0_u32);
    Router::new()
        .route(Method::Get, "/", index)
        .route(Method::Get, "/health", health)
        .route(Method::Post, "/echo", echo)
        .route(Method::Get, "/headers", headers)
        .route(Method::Get, "/hello", hello)
        .route(Method::Get, "/counter", move |_request| {
            counter.set(counter.get() + 1);
            Response::text(Status::Ok, &counter.get().to_string())
        })
}

fn index(_request: &Request) -> Response {
    Response::text(Status::Ok, "hello\n")
}

fn health(_request: &Request) -> Response {
    Response::text(Status::Ok, "ok\n")
}

fn echo(request: &Request) -> Response {
    Response::new(Status::Ok)
        .header(
            "Content-Type",
            request
                .headers
                .get("Content-Type")
                .unwrap_or("application/octet-stream"),
        )
        .body(request.body.clone())
}

fn headers(request: &Request) -> Response {
    let mut text = String::new();
    for (name, value) in request.headers.iter() {
        text.push_str(name);
        text.push_str(": ");
        text.push_str(value);
        text.push('\n');
    }
    Response::text(Status::Ok, &text)
}

fn hello(request: &Request) -> Response {
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
