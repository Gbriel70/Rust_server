mod support;

use support::{send_raw, start_server};

fn request(method: &str, path: &str) -> Vec<u8> {
    format!("{method} {path} HTTP/1.1\r\nHost: routing.test\r\n\r\n").into_bytes()
}

#[test]
fn root_and_health() {
    let addr = start_server();
    for (path, body) in [("/", "hello\n"), ("/health", "ok\n")] {
        let response = send_raw(addr, &request("GET", path));
        assert_eq!(response.status, "HTTP/1.1 200 OK");
        assert_eq!(response.body, body.as_bytes());
        assert_eq!(response.header("Content-Type"), "text/plain; charset=utf-8");
    }
}

#[test]
fn unknown_path() {
    assert_eq!(
        send_raw(start_server(), &request("GET", "/nope")).status,
        "HTTP/1.1 404 Not Found"
    );
}

#[test]
fn delete_health_has_get_and_head_in_allow() {
    let response = send_raw(start_server(), &request("DELETE", "/health"));
    assert_eq!(response.status, "HTTP/1.1 405 Method Not Allowed");
    assert_eq!(response.header("Allow"), "GET, HEAD");
}

#[test]
fn post_root_is_405() {
    let response = send_raw(start_server(), &request("POST", "/"));
    assert_eq!(response.status, "HTTP/1.1 405 Method Not Allowed");
    assert_eq!(response.header("Allow"), "GET, HEAD");
}

#[test]
fn get_echo_is_405() {
    let response = send_raw(start_server(), &request("GET", "/echo"));
    assert_eq!(response.status, "HTTP/1.1 405 Method Not Allowed");
    assert_eq!(response.header("Allow"), "POST");
}

#[test]
fn head_health_matches_get_headers_without_body() {
    let addr = start_server();
    let get = send_raw(addr, &request("GET", "/health"));
    let head = send_raw(addr, &request("HEAD", "/health"));
    assert_eq!(head.status, get.status);
    assert_eq!(head.headers, get.headers);
    assert_eq!(head.header("Content-Length"), "3");
    assert!(head.body.is_empty());
}

#[test]
fn head_errors_never_send_a_body() {
    let addr = start_server();
    for (path, status) in [
        ("/nope", "HTTP/1.1 404 Not Found"),
        ("/echo", "HTTP/1.1 405 Method Not Allowed"),
        ("/a%2Fb", "HTTP/1.1 400 Bad Request"),
        ("/hello?name=%zz", "HTTP/1.1 400 Bad Request"),
    ] {
        let head = send_raw(addr, &request("HEAD", path));
        let get = send_raw(addr, &request("GET", path));
        assert_eq!(head.status, status);
        assert_eq!(head.headers, get.headers);
        assert!(head.body.is_empty());
    }
    let head = send_raw(addr, b"HEAD /health HTTP/1.1\r\n\r\n");
    assert_eq!(head.status, "HTTP/1.1 400 Bad Request");
    assert_eq!(head.header("Content-Length"), "19");
    assert!(head.body.is_empty());
}

#[test]
fn echo_preserves_every_byte_and_content_type() {
    let addr = start_server();
    let body: Vec<u8> = (0..=255).collect();
    for content_type in [None, Some("application/x-test")] {
        let extra =
            content_type.map_or(String::new(), |value| format!("Content-Type: {value}\r\n"));
        let mut raw =
            format!("POST /echo HTTP/1.1\r\nHost: x\r\n{extra}Content-Length: 256\r\n\r\n")
                .into_bytes();
        raw.extend_from_slice(&body);
        let response = send_raw(addr, &raw);
        assert_eq!(response.status, "HTTP/1.1 200 OK");
        assert_eq!(
            response.header("Content-Type"),
            content_type.unwrap_or("application/octet-stream")
        );
        assert_eq!(response.body, body);
    }
}

#[test]
fn query_and_trailing_slash() {
    let addr = start_server();
    assert_eq!(
        send_raw(addr, &request("GET", "/health?x=1")).status,
        "HTTP/1.1 200 OK"
    );
    assert_eq!(
        send_raw(addr, &request("GET", "/health/")).status,
        "HTTP/1.1 404 Not Found"
    );
}

#[test]
fn hello_query() {
    let addr = start_server();
    for (query, expected) in [
        ("name=Gabriel", "hello, Gabriel"),
        ("name=%F0%9F%99%82", "hello, 🙂"),
    ] {
        let response = send_raw(addr, &request("GET", &format!("/hello?{query}")));
        assert_eq!(response.status, "HTTP/1.1 200 OK");
        assert_eq!(response.body, expected.as_bytes());
    }
    assert_eq!(
        send_raw(addr, &request("GET", "/hello?name=%zz")).status,
        "HTTP/1.1 400 Bad Request"
    );
}

#[test]
fn encoded_separator_is_400() {
    assert_eq!(
        send_raw(start_server(), &request("GET", "/a%2Fb")).status,
        "HTTP/1.1 400 Bad Request"
    );
}

#[test]
fn headers_contains_received_host_and_preserves_order() {
    let response = send_raw(
        start_server(),
        b"GET /headers HTTP/1.1\r\nHost: routing.test\r\nX-Test: a\r\nX-Test: b\r\n\r\n",
    );
    assert_eq!(response.status, "HTTP/1.1 200 OK");
    assert_eq!(response.body, b"Host: routing.test\nX-Test: a\nX-Test: b\n");
}

#[test]
fn counter_state_belongs_to_the_server_instance() {
    let addr = start_server();
    for expected in ["1", "2"] {
        let response = send_raw(addr, &request("GET", "/counter"));
        assert_eq!(response.status, "HTTP/1.1 200 OK");
        assert_eq!(response.body, expected.as_bytes());
    }
    let head = send_raw(addr, &request("HEAD", "/counter"));
    assert!(head.body.is_empty());
    assert_eq!(head.header("Content-Length"), "1");
    assert_eq!(send_raw(addr, &request("GET", "/counter")).body, b"4");
    assert_eq!(
        send_raw(start_server(), &request("GET", "/counter")).body,
        b"1"
    );
}
