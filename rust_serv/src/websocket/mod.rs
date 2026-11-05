//! RFC 6455 echo endpoint, independent framing and shared connection state.
pub mod codec;
pub mod session;
pub(crate) mod transport;

use crate::http::{
    borrowed_request::Request,
    request::{Method, Version},
    response::{Body, Response, Status},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha1::{Digest, Sha1};

pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_MESSAGE: usize = 1024 * 1024;
/// One echo frame plus a close frame, including both framing headers.
pub const MAX_OUTPUT: usize = MAX_MESSAGE + 10 + 127;

/// A successful response explicitly transfers the connection to WebSocket.
/// The HTTP body must be empty so no request payload becomes a frame by accident.
pub fn handshake(request: &Request<'_>) -> Response {
    let single = |name: &str| {
        let mut values = request
            .headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value);
        let value = values.next()?;
        values.next().is_none().then_some(value)
    };
    if request.method != Method::Get
        || request.version != Version::Http11
        || single("Host").is_none_or(|host| {
            host.is_empty()
                || host.bytes().any(|byte| byte.is_ascii_whitespace())
                || host.contains(['/', '\\', '@', '?', '#', ','])
        })
        || !request.headers.has_token("Connection", "upgrade")
        || !request.headers.has_token("Upgrade", "websocket")
        || !request.body.is_empty()
        || request.headers.get("Transfer-Encoding").is_some()
    {
        return Response::text(Status::BadRequest, "invalid WebSocket opening\n");
    }
    if single("Sec-WebSocket-Version") != Some("13") {
        return Response::text(Status::UpgradeRequired, "WebSocket version 13 required\n")
            .header("Upgrade", "websocket")
            .header("Connection", "Upgrade")
            .header("Sec-WebSocket-Version", "13");
    }
    let Some(key) = single("Sec-WebSocket-Key") else {
        return Response::text(Status::BadRequest, "one WebSocket key required\n");
    };
    if !matches!(STANDARD.decode(key), Ok(bytes) if bytes.len() == 16) {
        return Response::text(Status::BadRequest, "invalid WebSocket key\n");
    }
    let mut digest = Sha1::new();
    digest.update(key.as_bytes());
    digest.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    Response::new(Status::SwitchingProtocols)
        .header("Upgrade", "websocket")
        .header("Connection", "Upgrade")
        .header("Sec-WebSocket-Accept", STANDARD.encode(digest.finalize()))
        .body(Body::WebSocket)
}
