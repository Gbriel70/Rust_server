use rust_serv::server::Server;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::Duration;

pub fn start_server() -> SocketAddr {
    // Box<dyn Fn> is intentionally not Send. Construct it on the serving thread.
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {
        let server = Server::bind("127.0.0.1:0", rust_serv::routes::default_router()).unwrap();
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

pub struct RawResponse {
    pub status: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RawResponse {
    pub fn header(&self, name: &str) -> &str {
        &self
            .headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .unwrap()
            .1
    }
}

pub fn receive(mut stream: TcpStream, head_only: bool) -> RawResponse {
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
    let length = response.header("Content-Length").parse::<usize>().unwrap();
    if head_only {
        assert!(response.body.is_empty(), "HEAD must not send body bytes");
    } else {
        assert_eq!(length, response.body.len());
    }
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

pub fn send_raw(addr: SocketAddr, request: &[u8]) -> RawResponse {
    let mut stream = connect(addr);
    stream.write_all(request).unwrap();
    receive(stream, request.starts_with(b"HEAD "))
}
