//! Construction and serialization of HTTP responses.

use std::fmt;
use std::io::{self, Write};

pub use super::body::Body;
use super::body::{Chunk, encode_chunk};
use super::headers::Headers;

/// HTTP statuses that the server can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    PartialContent,
    RangeNotSatisfiable,
    NoContent,
    Forbidden,
    MovedPermanently,
    NotModified,
    InternalServerError,
    BadRequest,
    NotFound,
    MethodNotAllowed,
    RequestTimeout,
    PayloadTooLarge,
    RequestHeaderFieldsTooLarge,
    NotImplemented,
    HttpVersionNotSupported,
}

impl Status {
    /// Returns the numeric code associated with the HTTP status.
    pub fn code(self) -> u16 {
        match self {
            Self::Ok => 200,
            Self::PartialContent => 206,
            Self::RangeNotSatisfiable => 416,
            Self::NoContent => 204,
            Self::Forbidden => 403,
            Self::MovedPermanently => 301,
            Self::NotModified => 304,
            Self::InternalServerError => 500,
            Self::BadRequest => 400,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::RequestTimeout => 408,
            Self::PayloadTooLarge => 413,
            Self::RequestHeaderFieldsTooLarge => 431,
            Self::NotImplemented => 501,
            Self::HttpVersionNotSupported => 505,
        }
    }

    pub fn reason(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::PartialContent => "Partial Content",
            Self::RangeNotSatisfiable => "Range Not Satisfiable",
            Self::NoContent => "No Content",
            Self::Forbidden => "Forbidden",
            Self::MovedPermanently => "Moved Permanently",
            Self::NotModified => "Not Modified",
            Self::InternalServerError => "Internal Server Error",
            Self::BadRequest => "Bad Request",
            Self::NotFound => "Not Found",
            Self::MethodNotAllowed => "Method Not Allowed",
            Self::RequestTimeout => "Request Timeout",
            Self::PayloadTooLarge => "Payload Too Large",
            Self::RequestHeaderFieldsTooLarge => "Request Header Fields Too Large",
            Self::NotImplemented => "Not Implemented",
            Self::HttpVersionNotSupported => "HTTP Version Not Supported",
        }
    }
}

/// Writes the status in the format used by the response status line.
impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.code(), self.reason())
    }
}

/// HTTP response ready to be serialized to a stream.
#[derive(Debug)]
pub struct Response {
    /// Response status code.
    pub status: Status,
    /// Additional headers sent before the body.
    pub headers: Headers,
    /// Owned body source; file contents are read only by the transport.
    pub body: Body,
}

impl Response {
    /// Creates a response with no additional headers and an empty body.
    pub fn new(status: Status) -> Self {
        Self {
            status,
            headers: Headers::new(),
            body: Body::Empty,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push(name.into(), value.into());
        self
    }

    pub fn text(status: Status, text: &str) -> Self {
        Self::new(status)
            .header("Content-Type", "text/plain; charset=utf-8")
            .body(text)
    }

    /// Sets the response body and allows method chaining.
    pub fn body(mut self, bytes: impl Into<Body>) -> Self {
        self.body = bytes.into();
        self
    }

    /// Serializes the response and writes it to the destination.
    ///
    /// Validation occurs before anything is written. A user-provided
    /// framing header is ignored; Body chooses Content-Length or chunked.
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        self.write_response(w, true)
    }

    /// Serializes the same headers as GET, retaining its body length, without the body.
    pub fn write_head_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        self.write_response(w, false)
    }

    /// Serializes only the head. Framing headers belong to this serializer,
    /// never to application handlers (no Content-Length + Transfer-Encoding).
    pub fn write_headers<W: Write>(&self, w: &mut W) -> io::Result<()> {
        for (name, value) in self.headers.iter() {
            if name
                .bytes()
                .chain(value.bytes())
                .any(|b| matches!(b, b'\r' | b'\n' | 0))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid response header",
                ));
            }
        }
        let mut bytes = Vec::new();
        write!(bytes, "HTTP/1.1 {}\r\n", self.status)?;
        for (name, value) in self.headers.iter() {
            if !name.eq_ignore_ascii_case("Content-Length")
                && !name.eq_ignore_ascii_case("Transfer-Encoding")
            {
                write!(bytes, "{name}: {value}\r\n")?;
            }
        }
        if self.allows_body() {
            match self.body.len() {
                Some(len) => write!(bytes, "Content-Length: {len}\r\n")?,
                None => write!(bytes, "Transfer-Encoding: chunked\r\n")?,
            }
        }
        write!(bytes, "\r\n")?;
        w.write_all(&bytes)
    }
    pub fn allows_body(&self) -> bool {
        !matches!(self.status, Status::NoContent | Status::NotModified)
    }
    pub fn is_chunked(&self) -> bool {
        self.allows_body() && self.body.len().is_none()
    }
    fn write_response<W: Write>(&self, w: &mut W, include_body: bool) -> io::Result<()> {
        self.write_headers(w)?;
        if !include_body || !self.allows_body() {
            return Ok(());
        }
        let mut cursor = self.body.cursor()?;
        loop {
            match cursor.next_chunk()? {
                Chunk::Data(bytes) => {
                    if self.is_chunked() {
                        w.write_all(&encode_chunk(&bytes))?;
                    } else {
                        w.write_all(&bytes)?;
                    }
                }
                Chunk::Pending(ready) => {
                    std::thread::sleep(ready.saturating_duration_since(std::time::Instant::now()))
                }
                Chunk::Done => {
                    if self.is_chunked() {
                        w.write_all(b"0\r\n\r\n")?;
                    }
                    return Ok(());
                }
            }
        }
    }
}
