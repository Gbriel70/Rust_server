//! Types representing the request line and a complete HTTP request.

pub use super::headers::Headers;

use std::{fmt, str::FromStr};

/// Represents an HTTP request that has already been validated by the parser.
#[derive(Debug, PartialEq)]
pub struct Request {
    /// HTTP method used by the request.
    pub method: Method,
    /// Requested resource path, including an optional query string.
    pub target: String,
    /// Protocol version declared in the request line.
    pub version: Version,
    /// Received headers, preserving their order and repetitions.
    pub headers: Headers,
    /// Request body, without the headers or the HTTP terminator.
    pub body: Vec<u8>,
}

impl Request {
    /// Borrows the path without decoding it or normalizing its segments.
    pub fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_str(), |(path, _)| path)
    }

    /// Borrows everything after the first `?`, including an empty query.
    pub fn query(&self) -> Option<&str> {
        self.target.split_once('?').map(|(_, query)| query)
    }
}

/// HTTP methods accepted by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Method {
    type Err = ();

    /// Converts a textual method name to its internal representation.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "GET" => Ok(Method::Get),
            "POST" => Ok(Method::Post),
            "PUT" => Ok(Method::Put),
            "DELETE" => Ok(Method::Delete),
            "HEAD" => Ok(Method::Head),
            "OPTIONS" => Ok(Method::Options),
            _ => Err(()),
        }
    }
}

/// HTTP versions supported by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Http10,
    Http11,
}

impl FromStr for Version {
    type Err = ();

    /// Converts a textual version, such as `HTTP/1.1`, to `Version`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "HTTP/1.0" => Ok(Version::Http10),
            "HTTP/1.1" => Ok(Version::Http11),
            _ => Err(()),
        }
    }
}
