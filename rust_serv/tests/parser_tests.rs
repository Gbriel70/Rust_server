#[cfg(test)]
mod tests {
    use rust_serv::http::parser::{parse_request, ParseError};

    fn bytes(input: &str) -> &[u8] {
        input.as_bytes()
    }

    #[test]
    fn valid_get_http11_with_host() {
        let request =
            "GET / HTTP/1.1\r\n\
             Host: localhost\r\n\
             \r\n";

        let result = parse_request(bytes(request));

        let Ok(Some((_, consumed))) = result else {
            panic!("expected valid request");
        };

        assert_eq!(consumed, request.len());
    }

    #[test]
    fn http10_without_host() {
        let request =
            "GET / HTTP/1.0\r\n\
             \r\n";

        let result = parse_request(bytes(request));

        assert!(matches!(result, Ok(Some(_))));
    }

    #[test]
    fn http11_without_host() {
        let request =
            "GET / HTTP/1.1\r\n\
             \r\n";

        let result = parse_request(bytes(request));

        assert!(matches!(result, Err(ParseError::MissingHost)));
    }

    #[test]
    fn post_with_complete_body() {
        let request =
            "POST /data HTTP/1.1\r\n\
            Host: localhost\r\n\
            Content-Length: 5\r\n\
            \r\n\
            hello";

        let result = parse_request(bytes(request));

        let Ok(Some((parsed_request, consumed))) = result else {
            panic!("expected valid request");
        };

        assert_eq!(parsed_request.body, b"hello");
        assert_eq!(consumed, request.len());
    }

    #[test]
    fn post_with_incomplete_body() {
        let raw =
            "POST /data HTTP/1.1\r\n\
            Host: localhost\r\n\
            Content-Length: 5\r\n\
            \r\n\
            hel";

        let result = parse_request(bytes(raw));

        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn repeated_headers_are_preserved() {
        let raw =
            "GET / HTTP/1.1\r\n\
            Host: localhost\r\n\
            Accept: text/html\r\n\
            Accept: application/json\r\n\
            \r\n";

        let result = parse_request(bytes(raw));

        let Ok(Some((request, _))) = result else {
            panic!("expected valid request");
        };

        let accepts: Vec<&str> = request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("Accept"))
            .map(|(_, value)| value.as_str())
            .collect();

        assert_eq!(
            accepts,
            vec!["text/html", "application/json"]
        );
    }

    #[test]
    fn header_lookup_is_case_insensitive_and_value_is_trimmed() {
        let raw =
            "GET / HTTP/1.1\r\n\
            Host:   x  \r\n\
            \r\n";

        let result = parse_request(bytes(raw));

        let Ok(Some((request, _))) = result else {
            panic!("expected valid request");
        };

        assert_eq!(
            request.headers.get("HOST"),
            Some("x")
        );
    }

    #[test]
    fn lf_only_is_treated_as_incomplete() {
        let raw =
            "GET / HTTP/1.1\n\
            Host: localhost\n\
            \n";

        let result = parse_request(bytes(raw));

        assert!(matches!(result, Ok(None)));
    }
}