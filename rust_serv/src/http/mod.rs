//! Components for parsing HTTP requests and building HTTP responses.

pub mod body;
pub mod headers;
pub mod parser;
pub mod request;
pub mod response;
pub mod uri;

pub mod date;

pub mod range;

pub mod borrowed_request;

/// Borrowed request view used by the server. The owned compatibility API lives in request.
pub use borrowed_request::Request;
