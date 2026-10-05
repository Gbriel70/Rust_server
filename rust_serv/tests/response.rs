#[cfg(test)]
mod tests {
    use rust_serv::http::response::{Response, Status};
    use std::io;

    fn serialized(response: Response) -> Vec<u8> {
        let mut bytes = Vec::new();
        response.write_to(&mut bytes).unwrap();
        bytes
    }

    #[test]
    fn body() {
        assert_eq!(
            serialized(Response::new(Status::Ok).body(b"hello".to_vec())),
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello"
        );
    }

    #[test]
    fn empty_body() {
        assert_eq!(
            serialized(Response::new(Status::Ok)),
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn binary_body() {
        assert_eq!(
            serialized(Response::new(Status::Ok).body(vec![0xff, 0])),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n\xff\0"
        );
    }

    #[test]
    fn utf8_length_is_bytes() {
        assert_eq!(
            serialized(Response::new(Status::Ok).body("olá")),
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nolá".as_bytes()
        );
    }

    #[test]
    fn headers_keep_order() {
        assert_eq!(
            serialized(
                Response::new(Status::Ok)
                    .header("X-B", "2")
                    .header("X-A", "1")
            ),
            b"HTTP/1.1 200 OK\r\nX-B: 2\r\nX-A: 1\r\nContent-Length: 0\r\n\r\n"
        );
    }

    #[test]
    fn user_content_length_is_ignored() {
        assert_eq!(
            serialized(
                Response::new(Status::Ok)
                    .header("content-length", "999")
                    .header("Content-Length", "7")
                    .body("x")
            ),
            b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\nx"
        );
    }

    #[test]
    fn injection_is_rejected_before_writing() {
        for (name, value) in [
            ("X", "a\r\nb"),
            ("X\0", "a"),
            ("X\r", "a"),
            ("X\n", "a"),
            ("X", "a\0"),
            ("X", "a\n"),
            ("Content-Length", "1\r\nX: a"),
        ] {
            let mut bytes = Vec::new();
            let err = Response::new(Status::Ok)
                .header(name, value)
                .write_to(&mut bytes)
                .unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert!(bytes.is_empty());
        }
    }

    #[test]
    fn statuses() {
        for (status, code, reason) in [
            (Status::Ok, 200, "OK"),
            (Status::BadRequest, 400, "Bad Request"),
            (Status::PayloadTooLarge, 413, "Payload Too Large"),
            (
                Status::RequestHeaderFieldsTooLarge,
                431,
                "Request Header Fields Too Large",
            ),
            (Status::NotImplemented, 501, "Not Implemented"),
            (
                Status::HttpVersionNotSupported,
                505,
                "HTTP Version Not Supported",
            ),
        ] {
            assert_eq!(status.code(), code);
            assert_eq!(status.reason(), reason);
            assert_eq!(status.to_string(), format!("{code} {reason}"));
            assert_eq!(
                serialized(Response::new(status)),
                format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\n\r\n").as_bytes()
            );
        }
    }
}
