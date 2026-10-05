#[cfg(test)]
mod tests {
    use rust_serv::error::Error;
    use rust_serv::http::parser::ParseError;
    use std::error::Error as _;
    use std::io;

    #[test]
    fn conversions_preserve_sources() {
        let error = Error::from(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        assert_eq!(error.to_string(), "I/O error: closed");
        assert_eq!(error.source().unwrap().to_string(), "closed");
        let error = Error::from(ParseError::MissingHost);
        assert_eq!(error.to_string(), "HTTP parse error: missing Host header");
        assert_eq!(error.source().unwrap().to_string(), "missing Host header");
        assert!(Error::UnexpectedEof.source().is_none());
        assert_eq!(
            Error::UnexpectedEof.to_string(),
            "connection closed during request"
        );
    }
}
