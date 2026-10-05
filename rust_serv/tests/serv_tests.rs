#[cfg(test)]
mod tests {
    use rust_serv::http::parser::ParseError;
    use rust_serv::http::response::Status;
    use rust_serv::server::status_for;

    #[test]
    fn all_parse_errors_have_statuses() {
        for (error, status) in [
            (ParseError::MalformedRequestLine, Status::BadRequest),
            (ParseError::MalformedHeader, Status::BadRequest),
            (ParseError::MissingHost, Status::BadRequest),
            (ParseError::InvalidContentLength, Status::BadRequest),
            (ParseError::BodyTooLarge, Status::PayloadTooLarge),
            (
                ParseError::HeadersTooLarge,
                Status::RequestHeaderFieldsTooLarge,
            ),
            (ParseError::UnsupportedMethod, Status::NotImplemented),
            (
                ParseError::UnsupportedTransferEncoding,
                Status::NotImplemented,
            ),
            (
                ParseError::UnsupportedVersion,
                Status::HttpVersionNotSupported,
            ),
        ] {
            assert_eq!(status_for(&error), status);
            assert!(!error.to_string().is_empty());
        }
    }
}
