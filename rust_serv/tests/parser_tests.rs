#[cfg(test)]
mod tests {
    use rust_serv::http::parser::{MAX_BODY, MAX_HEAD, ParseError, parse_request};
    use rust_serv::http::request::{Method, Request, Version};

    const GET: &str = "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";

    type Parsed = Result<Option<(Request, usize)>, ParseError>;

    fn parse(raw: &str) -> Parsed {
        parse_request(raw.as_bytes())
    }

    fn parse_err(raw: &str) -> ParseError {
        parse(raw).unwrap_err()
    }

    fn post_with_length(value: &str) -> String {
        format!("POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {value}\r\n\r\n")
    }

    /// A valid GET whose head is exactly `len` bytes long.
    fn request_with_head_len(len: usize) -> String {
        let base = "GET / HTTP/1.1\r\nHost: x\r\nX: ";
        let tail = "\r\n\r\n";
        format!("{base}{}{tail}", "a".repeat(len - base.len() - tail.len()))
    }

    /// Feeds `raw` to the parser `chunk` bytes at a time, like a server loop would.
    fn parse_in_chunks(raw: &[u8], chunk: usize) -> Parsed {
        let mut buf = Vec::new();

        for piece in raw.chunks(chunk) {
            buf.extend_from_slice(piece);

            match parse_request(&buf) {
                Ok(None) => continue,
                other => return other,
            }
        }

        Ok(None)
    }

    // ---------- happy paths ----------

    #[test]
    fn simple_get() {
        let (req, consumed) = parse(GET).unwrap().unwrap();

        assert_eq!(consumed, GET.len());
        assert_eq!(req.method, Method::Get);
        assert_eq!(req.target, "/");
        assert_eq!(req.version, Version::Http11);
        assert_eq!(req.headers.get("host"), Some("example.com"));
        assert!(req.body.is_empty());
    }

    #[test]
    fn http10_does_not_need_host() {
        assert!(parse("GET / HTTP/1.0\r\n\r\n").unwrap().is_some());
    }

    #[test]
    fn options_asterisk_is_accepted() {
        let (req, _) = parse("OPTIONS * HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap()
            .unwrap();

        assert_eq!(req.method, Method::Options);
        assert_eq!(req.target, "*");
    }

    #[test]
    fn post_with_body() {
        let raw = "POST /x HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello";
        let (req, consumed) = parse(raw).unwrap().unwrap();

        assert_eq!(consumed, raw.len());
        assert_eq!(req.body, b"hello");
    }

    #[test]
    fn incomplete_body_is_none() {
        let raw = "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhel";

        assert_eq!(parse(raw), Ok(None));
    }

    #[test]
    fn two_glued_requests() {
        let second = "GET /b HTTP/1.1\r\nHost: y\r\n\r\n";
        let raw = format!("{GET}{second}");

        let (first, consumed) = parse(&raw).unwrap().unwrap();
        assert_eq!(consumed, GET.len());
        assert_eq!(first.target, "/");

        let (next, consumed_next) = parse_request(&raw.as_bytes()[consumed..]).unwrap().unwrap();
        assert_eq!(next.target, "/b");
        assert_eq!(consumed_next, second.len());
    }

    #[test]
    fn headers_are_trimmed_case_insensitive_and_keep_order() {
        let raw = "GET / HTTP/1.1\r\nHost:   x  \r\nAccept: a\r\nAccept: b\r\n\r\n";
        let (req, _) = parse(raw).unwrap().unwrap();

        assert_eq!(req.headers.get("HOST"), Some("x"));

        let accepts: Vec<&str> = req
            .headers
            .iter()
            .filter(|(name, _)| name.as_str() == "Accept")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(accepts, ["a", "b"]);
    }

    #[test]
    fn bare_lf_is_never_a_terminator() {
        // Deliberate strictness: without CRLF CRLF the request is simply incomplete.
        assert_eq!(parse("GET / HTTP/1.1\nHost: x\n\n"), Ok(None));
    }

    // ---------- fragmentation ----------

    #[test]
    fn every_proper_prefix_is_incomplete() {
        let raw = b"POST /x HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello";

        for i in 0..raw.len() {
            assert_eq!(parse_request(&raw[..i]), Ok(None), "prefix of {i} bytes");
        }

        let (req, consumed) = parse_request(raw).unwrap().unwrap();
        assert_eq!(consumed, raw.len());
        assert_eq!(req.body, b"hello");
    }

    #[test]
    fn result_does_not_depend_on_chunking() {
        let big_head = request_with_head_len(MAX_HEAD + 1);
        let samples: Vec<Vec<u8>> = vec![
            GET.as_bytes().to_vec(),
            b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello".to_vec(),
            b"GET / HTTP/1.1\r\nHost: x\r\nBroken\r\n\r\n".to_vec(),
            post_with_length("abc").into_bytes(),
            big_head.into_bytes(),
            format!("{GET}{GET}").into_bytes(),
        ];

        for raw in &samples {
            let expected = parse_request(raw);

            for chunk in [1, 2, 7, 16, 1024] {
                assert_eq!(
                    parse_in_chunks(raw, chunk),
                    expected,
                    "chunk size {chunk}, input {:?}",
                    String::from_utf8_lossy(raw)
                );
            }
        }
    }

    // ---------- request line ----------

    #[test]
    fn request_line_errors() {
        let cases = [
            ("GET /\r\nHost: x\r\n\r\n", ParseError::MalformedRequestLine),
            (
                "GET  / HTTP/1.1\r\nHost: x\r\n\r\n",
                ParseError::MalformedRequestLine,
            ),
            (
                "GET / HTTP/1.1 \r\nHost: x\r\n\r\n",
                ParseError::MalformedRequestLine,
            ),
            ("\r\n\r\n", ParseError::MalformedRequestLine),
            (
                "GET x HTTP/1.1\r\nHost: x\r\n\r\n",
                ParseError::MalformedRequestLine,
            ),
            (
                "GET * HTTP/1.1\r\nHost: x\r\n\r\n",
                ParseError::MalformedRequestLine,
            ),
            (
                "BREW / HTTP/1.1\r\nHost: x\r\n\r\n",
                ParseError::UnsupportedMethod,
            ),
            (
                "get / HTTP/1.1\r\nHost: x\r\n\r\n",
                ParseError::UnsupportedMethod,
            ),
            (
                "GET / HTTP/2.0\r\nHost: x\r\n\r\n",
                ParseError::UnsupportedVersion,
            ),
            (
                "GET / HTTP/1.1x\r\nHost: x\r\n\r\n",
                ParseError::UnsupportedVersion,
            ),
        ];

        for (raw, expected) in cases {
            assert_eq!(parse_err(raw), expected, "input: {raw:?}");
        }
    }

    #[test]
    fn target_rejects_control_and_non_ascii_bytes() {
        let targets = ["/a\nb", "/a\tb", "/a\0b", "/a\x7fb", "/caf\u{e9}"];

        for target in targets {
            let raw = format!("GET {target} HTTP/1.1\r\nHost: x\r\n\r\n");

            assert_eq!(
                parse_err(&raw),
                ParseError::MalformedRequestLine,
                "target: {target:?}"
            );
        }
    }

    // ---------- headers ----------

    #[test]
    fn header_errors() {
        let cases = [
            "GET / HTTP/1.1\r\nHost: x\r\nBroken\r\n\r\n",
            "GET / HTTP/1.1\r\nHost : x\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\n Folded: y\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nFoo Bar: y\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\n: y\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nX: a\0b\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nX: a\nb\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nX: a\rb\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nX: a\x7fb\r\n\r\n",
            "GET / HTTP/1.1\r\nHost: x\r\nFoo\0Bar: y\r\n\r\n",
        ];

        for raw in cases {
            assert_eq!(
                parse_err(raw),
                ParseError::MalformedHeader,
                "input: {raw:?}"
            );
        }
    }

    #[test]
    fn non_utf8_head_is_rejected() {
        let raw = b"GET / HTTP/1.1\r\nHost: \xff\r\n\r\n";

        assert_eq!(parse_request(raw), Err(ParseError::MalformedHeader));
    }

    #[test]
    fn host_rules() {
        assert_eq!(parse_err("GET / HTTP/1.1\r\n\r\n"), ParseError::MissingHost);
        assert_eq!(
            parse_err("GET / HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n"),
            ParseError::MalformedHeader
        );
    }

    // ---------- Content-Length / Transfer-Encoding ----------

    #[test]
    fn invalid_content_length_values() {
        for value in ["abc", "-1", "+5", "", "0x5", "5.0", "5 5"] {
            assert_eq!(
                parse_err(&post_with_length(value)),
                ParseError::InvalidContentLength,
                "value: {value:?}"
            );
        }
    }

    #[test]
    fn duplicate_content_length_is_rejected() {
        let raw =
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello";

        assert_eq!(parse_err(raw), ParseError::InvalidContentLength);
    }

    #[test]
    fn content_length_overflow_is_body_too_large() {
        assert_eq!(
            parse_err(&post_with_length("99999999999999999999999")),
            ParseError::BodyTooLarge
        );
    }

    #[test]
    fn transfer_encoding_wins_over_content_length() {
        let raw = "POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: abc\r\n\r\n";

        assert_eq!(parse_err(raw), ParseError::UnsupportedTransferEncoding);
    }

    // ---------- limits ----------

    #[test]
    fn head_limit_boundaries() {
        assert!(parse(&request_with_head_len(MAX_HEAD)).unwrap().is_some());
        assert_eq!(
            parse_err(&request_with_head_len(MAX_HEAD + 1)),
            ParseError::HeadersTooLarge
        );
    }

    #[test]
    fn oversized_head_without_terminator() {
        assert_eq!(parse(&"a".repeat(MAX_HEAD)), Ok(None));
        assert_eq!(
            parse_err(&"a".repeat(MAX_HEAD + 1)),
            ParseError::HeadersTooLarge
        );
    }

    #[test]
    fn header_bomb() {
        let raw = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(20_000));

        assert_eq!(parse_err(&raw), ParseError::HeadersTooLarge);
    }

    #[test]
    fn body_limit_boundaries() {
        // Exactly MAX_BODY: valid, but incomplete until the whole body arrives.
        let head = post_with_length(&MAX_BODY.to_string());
        assert_eq!(parse(&head), Ok(None));

        let mut raw = head.into_bytes();
        raw.extend(vec![b'a'; MAX_BODY]);
        let (req, consumed) = parse_request(&raw).unwrap().unwrap();
        assert_eq!(req.body.len(), MAX_BODY);
        assert_eq!(consumed, raw.len());

        // MAX_BODY + 1 fails immediately, without waiting for any body bytes.
        assert_eq!(
            parse_err(&post_with_length(&(MAX_BODY + 1).to_string())),
            ParseError::BodyTooLarge
        );
    }
}
