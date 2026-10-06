//! Construction and serialization of HTTP responses.

use std::fmt;
use std::io::{self, Write};

use super::headers::Headers;

/// HTTP statuses that the server can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
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
    /// Response body contents.
    pub body: Vec<u8>,
}

impl Response {
    /// Creates a response with no additional headers and an empty body.
    pub fn new(status: Status) -> Self {
        Self {
            status,
            headers: Headers::new(),
            body: Vec::new(),
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
    pub fn body(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.body = bytes.into();
        self
    }

    /// Serializes the response and writes it to the destination.
    ///
    /// Validation occurs before anything is written. A user-provided
    /// `Content-Length` is ignored; the body's actual byte length is always
    /// used to frame the response.
    pub fn write_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        self.write_response(w, true)
    }

    /// Serializes the same headers as GET, retaining its body length, without the body.
    pub fn write_head_to<W: Write>(&self, w: &mut W) -> io::Result<()> {
        self.write_response(w, false)
    }

    fn write_response<W: Write>(&self, w: &mut W, include_body: bool) -> io::Result<()> {
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
            if !name.eq_ignore_ascii_case("Content-Length") {
                write!(bytes, "{name}: {value}\r\n")?;
            }
        }
        let no_body = matches!(self.status, Status::NoContent | Status::NotModified);
        if !no_body {
            write!(bytes, "Content-Length: {}\r\n", self.body.len())?;
        }
        write!(bytes, "\r\n")?;
        if include_body && !no_body {
            bytes.extend_from_slice(&self.body);
        }
        w.write_all(&bytes)
    }
}
