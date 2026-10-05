//! Error types shared by the parser and the TCP server.

use std::{fmt, io};

use crate::http::parser::ParseError;

/// Errors that can occur while the server receives and processes a connection.
#[derive(Debug)]
pub enum Error {
    /// Socket read or write failure.
    Io(io::Error),
    /// The received request does not follow the accepted HTTP format.
    Parse(ParseError),
    /// The connection ended before an incomplete request was finished.
    UnexpectedEof,
}

/// Defines messages displayed when logging or returning an error.
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O error: {error}"),
            Self::Parse(error) => write!(f, "HTTP parse error: {error}"),
            Self::UnexpectedEof => f.write_str("connection closed during request"),
        }
    }
}

/// Preserves the original cause for diagnostics through `source()`.
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::UnexpectedEof => None,
        }
    }
}

/// Converts I/O errors into the server's unified error type.
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Converts parser errors into the server's unified error type.
impl From<ParseError> for Error {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}
