use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

use rust_serv::http::parser::{MAX_BODY, MAX_HEAD};
use rust_serv::server::Server;

fn start_server() -> SocketAddr {
    let server = Server::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    thread::spawn(move || server.run().unwrap());
    addr
}

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
}

struct RawResponse {
    status: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl RawResponse {
    fn header(&self, name: &str) -> &str {
        &self
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .unwrap()
            .1
    }
}

fn receive(mut stream: TcpStream) -> RawResponse {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let mut lines = std::str::from_utf8(&bytes[..end]).unwrap().split("\r\n");
    let status = lines.next().unwrap().to_owned();
    let headers = lines
        .map(|line| {
            let (name, value) = line.split_once(": ").unwrap();
            (name.to_owned(), value.to_owned())
        })
        .collect();
    let response = RawResponse {
        status,
        headers,
        body: bytes[end + 4..].to_vec(),
    };
    assert_eq!(
        response.header("Content-Length").parse::<usize>().unwrap(),
        response.body.len()
    );
    assert_eq!(
        response
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
            .count(),
        1
    );
    assert_eq!(response.header("Connection"), "close");
    response
}

fn send_raw(addr: SocketAddr, request: &[u8]) -> RawResponse {
    let mut stream = connect(addr);
    stream.write_all(request).unwrap();
    receive(stream)
}

const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";

#[test]
fn valid_get() {
    let response = send_raw(start_server(), GET);
    assert_eq!(response.status, "HTTP/1.1 200 OK");
    assert_eq!(response.body, b"hello");
}

macro_rules! error_test {
    ($name:ident, $request:expr, $status:literal, $body:literal) => {
        #[test]
        fn $name() {
            let response = send_raw(start_server(), $request);
            assert_eq!(response.status, $status);
            assert_eq!(response.body, $body.as_bytes());
        }
    };
}

error_test!(
    malformed_request_line,
    b"GET /\r\n\r\n",
    "HTTP/1.1 400 Bad Request",
    "malformed request line"
);
error_test!(
    malformed_header,
    b"GET / HTTP/1.1\r\nHost: x\r\nBroken\r\n\r\n",
    "HTTP/1.1 400 Bad Request",
    "malformed header"
);
error_test!(
    missing_host,
    b"GET / HTTP/1.1\r\n\r\n",
    "HTTP/1.1 400 Bad Request",
    "missing Host header"
);
error_test!(
    unsupported_version,
    b"GET / HTTP/2.0\r\nHost: x\r\n\r\n",
    "HTTP/1.1 505 HTTP Version Not Supported",
    "unsupported HTTP version"
);
error_test!(
    unsupported_method,
    b"BREW / HTTP/1.1\r\nHost: x\r\n\r\n",
    "HTTP/1.1 501 Not Implemented",
    "unsupported method"
);
error_test!(
    transfer_encoding,
    b"GET / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
    "HTTP/1.1 501 Not Implemented",
    "unsupported Transfer-Encoding"
);
error_test!(
    invalid_content_length,
    b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: abc\r\n\r\n",
    "HTTP/1.1 400 Bad Request",
    "invalid Content-Length"
);

#[test]
fn oversized_body_rejected_without_waiting() {
    let request = format!(
        "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY + 1
    );
    let response = send_raw(start_server(), request.as_bytes());
    assert_eq!(response.status, "HTTP/1.1 413 Payload Too Large");
    assert_eq!(response.body, b"request body too large");
}

#[test]
fn oversized_head() {
    let mut request = b"GET / HTTP/1.1\r\nX: ".to_vec();
    request.resize(MAX_HEAD + 8, b'a');
    let response = send_raw(start_server(), &request);
    assert_eq!(
        response.status,
        "HTTP/1.1 431 Request Header Fields Too Large"
    );
    assert_eq!(response.body, b"request headers too large");
}

#[test]
fn fragmented_request() {
    let mut stream = connect(start_server());
    stream.write_all(&GET[..10]).unwrap();
    thread::sleep(Duration::from_millis(50));
    stream.write_all(&GET[10..]).unwrap();
    assert_eq!(receive(stream).status, "HTTP/1.1 200 OK");
}

fn close_then_request(partial: &[u8]) {
    let addr = start_server();
    let mut stream = connect(addr);
    stream.write_all(partial).unwrap();
    stream.shutdown(Shutdown::Write).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    assert!(bytes.is_empty());
    assert_eq!(send_raw(addr, GET).status, "HTTP/1.1 200 OK");
}

#[test]
fn empty_connection() {
    close_then_request(b"");
}

#[test]
fn incomplete_request() {
    close_then_request(b"GET / HTTP/1.1\r\nHost:");
}

#[test]
fn post_body() {
    let response = send_raw(
        start_server(),
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\n\xff\0a",
    );
    assert_eq!(response.status, "HTTP/1.1 200 OK");
    assert_eq!(response.body, b"hello");
}
