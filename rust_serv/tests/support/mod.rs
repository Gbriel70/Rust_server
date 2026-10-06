// Integration test crates use different subsets of these shared helpers.
#![allow(dead_code)]

use rust_serv::connection::Config;
use rust_serv::server::Server;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

pub fn test_config() -> Config {
    Config {
        idle_timeout: Duration::from_millis(200),
        request_timeout: Duration::from_millis(200),
        write_timeout: Duration::from_millis(200),
        ..Config::default()
    }
}

pub fn start_server() -> SocketAddr {
    // Earlier phases test a single response and EOF; keep that explicit.
    start_server_with_config(Config {
        max_requests: 1,
        ..test_config()
    })
}

pub fn start_server_with_config(config: Config) -> SocketAddr {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        // Construct non-Send handlers on the serving thread.
        let server =
            Server::bind_with_config("127.0.0.1:0", rust_serv::routes::default_router(), config)
                .unwrap();
        sender.send(server.local_addr().unwrap()).unwrap();
        server.run().unwrap();
    });
    receiver.recv_timeout(Duration::from_secs(2)).unwrap()
}

pub fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
}

#[derive(Debug)]
pub struct RawResponse {
    pub status: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawResponse {
    pub fn header(&self, name: &str) -> &str {
        self.optional_header(name).unwrap()
    }

    pub fn optional_header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Reads a single Content-Length response and retains any following response bytes.
pub struct ResponseReader {
    pub stream: TcpStream,
    buf: Vec<u8>,
}

impl ResponseReader {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    pub fn send(&mut self, request: &[u8]) {
        self.stream.write_all(request).unwrap();
    }

    pub fn read_response(&mut self, head_only: bool) -> RawResponse {
        let end = loop {
            if let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break end;
            }
            assert!(self.buf.len() < 64 * 1024, "response head too large");
            self.read_more();
        };
        let mut lines = std::str::from_utf8(&self.buf[..end]).unwrap().split("\r\n");
        let status = lines.next().unwrap().to_owned();
        assert!(
            status.starts_with("HTTP/1.1 "),
            "response is misaligned: {status:?}"
        );
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(": ").unwrap();
                (name.to_owned(), value.to_owned())
            })
            .collect();
        let mut response = RawResponse {
            status,
            headers,
            body: Vec::new(),
        };
        let no_body = matches!(
            response.status.split_whitespace().nth(1),
            Some("204" | "304")
        ) || response
            .status
            .split_whitespace()
            .nth(1)
            .is_some_and(|s| s.starts_with('1'));
        let length = response
            .optional_header("Content-Length")
            .map_or(0, |s| s.parse::<usize>().unwrap());
        assert_eq!(
            response
                .headers
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
                .count(),
            usize::from(!no_body)
        );
        let body_length = if head_only || no_body { 0 } else { length };
        let consumed = end + 4 + body_length;
        while self.buf.len() < consumed {
            self.read_more();
        }
        response
            .body
            .extend_from_slice(&self.buf[end + 4..consumed]);
        self.buf.drain(..consumed);
        assert_eq!(response.body.len(), body_length);
        response
    }

    fn read_more(&mut self) {
        let mut chunk = [0; 4096];
        let n = self.stream.read(&mut chunk).unwrap();
        assert_ne!(n, 0, "EOF before a complete response");
        self.buf.extend_from_slice(&chunk[..n]);
    }

    pub fn assert_eof(&mut self) {
        assert!(
            self.buf.is_empty(),
            "unexpected bytes after final response: {:?}",
            self.buf
        );
        let mut byte = [0];
        assert_eq!(
            self.stream.read(&mut byte).unwrap(),
            0,
            "expected FIN without extra body bytes"
        );
    }
}

pub fn receive(stream: TcpStream, head_only: bool) -> RawResponse {
    let mut reader = ResponseReader::new(stream);
    let response = reader.read_response(head_only);
    assert_eq!(response.header("Connection"), "close");
    reader.assert_eof();
    response
}

pub fn send_raw(addr: SocketAddr, request: &[u8]) -> RawResponse {
    let mut stream = connect(addr);
    stream.write_all(request).unwrap();
    receive(stream, request.starts_with(b"HEAD "))
}
