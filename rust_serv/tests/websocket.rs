mod support;
use proptest::prelude::*;
use rust_serv::{
    connection::Config,
    http::{
        parser::parse_borrowed,
        response::{Body, Response, Status},
    },
    routes,
    server::{Execution, Server, ShutdownHandle},
    stats::Stats,
    websocket::{
        self, MAX_FRAME, MAX_MESSAGE,
        codec::{Frame, Opcode, ProtocolError, validate_close},
        session::Session,
    },
};
use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const OPEN: &[u8] = b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: keep-alive, Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
fn masked(opcode: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
    let mask = [0x12, 0x34, 0x56, 0x78];
    let mut bytes = vec![(u8::from(fin) * 128) | opcode];
    match payload.len() {
        n @ 0..=125 => bytes.push(128 | n as u8),
        n @ 126..=65535 => {
            bytes.push(254);
            bytes.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            bytes.push(255);
            bytes.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    bytes.extend_from_slice(&mask);
    bytes.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    bytes
}
fn read_head(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        assert!(bytes.len() < 8192);
    }
    String::from_utf8(bytes).unwrap()
}
fn read_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    let mut head = [0; 2];
    stream.read_exact(&mut head).unwrap();
    assert_eq!(head[0] & 0xf0, 0x80);
    assert_eq!(head[1] & 128, 0, "server frames must be unmasked");
    let length = match head[1] {
        126 => {
            let mut size = [0; 2];
            stream.read_exact(&mut size).unwrap();
            u16::from_be_bytes(size) as usize
        }
        127 => {
            let mut size = [0; 8];
            stream.read_exact(&mut size).unwrap();
            u64::from_be_bytes(size) as usize
        }
        n => n as usize,
    };
    assert!(length <= MAX_MESSAGE);
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).unwrap();
    (head[0] & 15, payload)
}
fn modes() -> Vec<Execution> {
    let mut modes = vec![
        Execution::ThreadPerConnection,
        Execution::Pool {
            workers: 2,
            queue_capacity: 4,
        },
        Execution::Tokio {
            max_connections: 32,
        },
    ];
    #[cfg(target_os = "linux")]
    modes.extend([
        Execution::Epoll {
            edge_triggered: false,
        },
        Execution::Epoll {
            edge_triggered: true,
        },
    ]);
    modes
}
fn config() -> Config {
    Config {
        idle_timeout: Duration::from_secs(3),
        request_timeout: Duration::from_secs(2),
        write_timeout: Duration::from_millis(400),
        max_requests: 100,
    }
}
struct Running {
    addr: SocketAddr,
    shutdown: ShutdownHandle,
    stats: Arc<Stats>,
    join: Option<JoinHandle<io::Result<()>>>,
}
impl Running {
    fn new(mode: Execution, config: Config) -> Self {
        let server = Server::bind_with_config("127.0.0.1:0", routes::default_router(), config)
            .unwrap()
            .with_execution(mode)
            .unwrap();
        let addr = server.local_addr().unwrap();
        let shutdown = server.shutdown_handle();
        let stats = server.stats();
        let join = Some(thread::spawn(move || server.run()));
        Self {
            addr,
            shutdown,
            stats,
            join,
        }
    }
    fn open(&self) -> TcpStream {
        let mut stream = support::connect(self.addr);
        stream.write_all(OPEN).unwrap();
        assert!(read_head(&mut stream).starts_with("HTTP/1.1 101"));
        stream
    }
    fn stop(&mut self) {
        self.shutdown.request_shutdown();
        if let Some(join) = self.join.take() {
            join.join().unwrap().unwrap();
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}
fn close(stream: &mut TcpStream) {
    stream
        .write_all(&masked(8, true, &1000u16.to_be_bytes()))
        .unwrap();
    assert_eq!(read_frame(stream), (8, 1000u16.to_be_bytes().to_vec()));
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
}
fn health(server: &Running) {
    let response = support::send_raw(
        server.addr,
        b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(response.status, "HTTP/1.1 200 OK");
    assert_eq!(response.body, b"ok\n");
}

#[test]
fn rfc_handshake_and_101_serialization() {
    let request = parse_borrowed(OPEN).unwrap().unwrap().0;
    let response = websocket::handshake(&request);
    assert!(response.is_websocket());
    assert_eq!(
        response.headers.get("Sec-WebSocket-Accept"),
        Some("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
    );
    let mut bytes = Vec::new();
    response.write_to(&mut bytes).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
    assert!(!text.contains("Content-Length"));
    assert!(!text.contains("Transfer-Encoding"));
    assert!(
        !Response::new(Status::Ok)
            .body(Body::WebSocket)
            .is_websocket()
    );
}
#[test]
fn handshake_rejects_invalid_and_duplicate_headers() {
    let opening = std::str::from_utf8(OPEN).unwrap();
    for (from, to, status) in [
        ("GET", "HEAD", Status::BadRequest),
        ("Host: localhost", "Host:", Status::BadRequest),
        ("Host: localhost", "Host: local host", Status::BadRequest),
        ("HTTP/1.1", "HTTP/1.0", Status::BadRequest),
        ("Upgrade: websocket", "Upgrade: other", Status::BadRequest),
        ("keep-alive, Upgrade", "keep-alive", Status::BadRequest),
        ("Version: 13", "Version: 12", Status::UpgradeRequired),
        (
            "Version: 13",
            "Version: 13\r\nSec-WebSocket-Version: 13",
            Status::UpgradeRequired,
        ),
        ("dGhlIHNhbXBsZSBub25jZQ==", "YQ==", Status::BadRequest),
        (
            "dGhlIHNhbXBsZSBub25jZQ==",
            "!!!!!!!!!!!!!!!!!!!!!!!!",
            Status::BadRequest,
        ),
        ("Sec-WebSocket-Key:", "Different:", Status::BadRequest),
        (
            "Key: dGhlIHNhbXBsZSBub25jZQ==",
            "Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==",
            Status::BadRequest,
        ),
        (
            "\r\n\r\n",
            "\r\nContent-Length: 1\r\n\r\nx",
            Status::BadRequest,
        ),
        (
            "\r\n\r\n",
            "\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            Status::BadRequest,
        ),
    ] {
        let wire = opening.replace(from, to);
        let request = parse_borrowed(wire.as_bytes()).unwrap().unwrap().0;
        let response = websocket::handshake(&request);
        assert_eq!(response.status, status, "{wire}");
        if status == Status::UpgradeRequired {
            assert_eq!(response.headers.get("Sec-WebSocket-Version"), Some("13"));
        }
    }
}
#[test]
fn codec_lengths_fragmentation_and_consumption() {
    for length in [0, 1, 125, 126, 65535, 65536, MAX_FRAME] {
        let payload = vec![0xa5; length];
        let wire = masked(2, true, &payload);
        for prefix in [0, 1, 2, 3, wire.len() / 2, wire.len() - 1] {
            assert_eq!(Frame::decode(&wire[..prefix]).unwrap(), None);
        }
        let (frame, consumed) = Frame::decode(&wire).unwrap().unwrap();
        assert_eq!(consumed, wire.len());
        assert_eq!(frame.payload, payload);
        let encoded = frame.encode();
        assert_eq!(encoded[1] & 128, 0);
        assert!(encoded.ends_with(&payload));
    }
}
#[test]
fn codec_rejects_headers_before_waiting_for_payload() {
    for bytes in [
        &[0x81, 0][..],
        &[0xf1, 128],
        &[0x83, 128],
        &[0x09, 128],
        &[0x89, 254],
        &[0x81, 254, 0, 125],
        &[0x81, 255, 0, 0, 0, 0, 0, 0, 255, 255],
        &[0x81, 255, 128, 0, 0, 0, 0, 0, 0, 0],
    ] {
        assert_eq!(Frame::decode(bytes), Err(ProtocolError::Protocol));
    }
    let mut big = vec![0x82, 255];
    big.extend_from_slice(&((MAX_FRAME + 1) as u64).to_be_bytes());
    assert_eq!(Frame::decode(&big), Err(ProtocolError::TooLarge));
}
#[test]
fn message_state_utf8_control_and_size_limits() {
    let mut session = Session::default();
    let frame = |opcode, fin, payload: &[u8]| Frame {
        opcode,
        fin,
        payload: payload.to_vec(),
    };
    assert_eq!(
        session.receive(frame(Opcode::Text, false, &[0xe2])),
        Ok(None)
    );
    assert_eq!(
        session
            .receive(frame(Opcode::Ping, true, b"check"))
            .unwrap()
            .unwrap()
            .opcode,
        Opcode::Pong
    );
    assert_eq!(
        session
            .receive(frame(Opcode::Continuation, true, &[0x82, 0xac]))
            .unwrap()
            .unwrap()
            .payload,
        "€".as_bytes()
    );
    assert_eq!(
        session.receive(frame(Opcode::Continuation, true, b"x")),
        Err(ProtocolError::Protocol)
    );
    assert_eq!(
        session.receive(frame(Opcode::Text, true, &[0xff])),
        Err(ProtocolError::InvalidUtf8)
    );
    session
        .receive(frame(Opcode::Binary, false, &vec![0; MAX_MESSAGE]))
        .unwrap();
    assert_eq!(
        session.receive(frame(Opcode::Continuation, true, b"x")),
        Err(ProtocolError::TooLarge)
    );
    session.start_close(1001).unwrap();
    assert_eq!(
        session.receive(frame(Opcode::Text, true, b"ignored")),
        Ok(None)
    );
    assert!(session.start_close(1001).is_none());
}
#[test]
fn close_payload_rules() {
    assert!(validate_close(&[]).is_ok());
    assert_eq!(validate_close(&[1]), Err(ProtocolError::Protocol));
    for code in [999u16, 1004, 1005, 1006, 1015, 1016, 2000, 2999, 5000] {
        assert_eq!(
            validate_close(&code.to_be_bytes()),
            Err(ProtocolError::Protocol)
        );
    }
    for code in [1000u16, 1001, 1002, 1003, 1007, 1011, 1014, 3000, 4999] {
        validate_close(&code.to_be_bytes()).unwrap();
    }
    let mut payload = 1000u16.to_be_bytes().to_vec();
    payload.push(255);
    assert_eq!(validate_close(&payload), Err(ProtocolError::InvalidUtf8));
}
proptest! {
    #[test]
    fn arbitrary_frames_never_panic_or_overconsume(input in prop::collection::vec(any::<u8>(), 0..4096)) {
        if let Ok(Some((frame, consumed))) = Frame::decode(&input) {
            prop_assert!(consumed <= input.len()); prop_assert!(frame.payload.len() <= MAX_FRAME);
            let mut session = Session::default(); let _ = session.receive(frame);
        }
    }
    #[test]
    fn masked_binary_round_trip(payload in prop::collection::vec(any::<u8>(), 0..8192)) {
        let wire = masked(2, true, &payload); let (frame, consumed) = Frame::decode(&wire).unwrap().unwrap();
        prop_assert_eq!(consumed, wire.len()); prop_assert_eq!(frame.payload, payload);
    }
}
#[test]
fn upgrade_retains_first_frames_and_avoids_http_framing_all_modes() {
    for mode in modes() {
        let server = Running::new(
            mode,
            Config {
                max_requests: 1,
                ..config()
            },
        );
        let mut stream = support::connect(server.addr);
        let mut wire = OPEN.to_vec();
        wire.extend(masked(1, true, b"coalesced"));
        wire.extend(masked(2, true, &[0, 255]));
        stream.write_all(&wire).unwrap();
        let head = read_head(&mut stream);
        assert!(head.starts_with("HTTP/1.1 101"), "{mode:?} {head}");
        assert!(!head.contains("Content-Length"));
        assert!(!head.contains("Transfer-Encoding"));
        assert!(!head.contains("Connection: close"));
        assert_eq!(read_frame(&mut stream), (1, b"coalesced".to_vec()));
        assert_eq!(read_frame(&mut stream), (2, vec![0, 255]));
        close(&mut stream);
        health(&server);
    }
}
#[test]
fn upgrade_after_http_keep_alive_preserves_pipeline_all_modes() {
    for mode in modes() {
        let server = Running::new(mode, config());
        let mut stream = support::connect(server.addr);
        let mut wire = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n".to_vec();
        wire.extend_from_slice(OPEN);
        wire.extend(masked(1, true, b"after HTTP"));
        stream.write_all(&wire).unwrap();
        assert!(read_head(&mut stream).starts_with("HTTP/1.1 200"));
        let mut body = [0; 3];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(&body, b"ok\n");
        assert!(read_head(&mut stream).starts_with("HTTP/1.1 101"));
        assert_eq!(read_frame(&mut stream), (1, b"after HTTP".to_vec()));
        close(&mut stream);
    }
}
#[test]
fn fragmented_handshake_frames_and_control_interleaving_all_modes() {
    for mode in modes() {
        let server = Running::new(mode, config());
        let mut stream = support::connect(server.addr);
        for chunk in OPEN.chunks(7) {
            stream.write_all(chunk).unwrap();
        }
        assert!(read_head(&mut stream).starts_with("HTTP/1.1 101"));
        let mut frames = masked(1, false, &[0xe2]);
        frames.extend(masked(9, true, b"ping"));
        frames.extend(masked(0, true, &[0x82, 0xac]));
        frames.extend(masked(10, true, b"pong"));
        frames.extend(masked(2, true, b""));
        for chunk in frames.chunks(3) {
            stream.write_all(chunk).unwrap();
        }
        assert_eq!(read_frame(&mut stream), (10, b"ping".to_vec()));
        assert_eq!(read_frame(&mut stream), (1, "€".as_bytes().to_vec()));
        assert_eq!(read_frame(&mut stream), (2, vec![]));
        let payload = [1000u16.to_be_bytes().as_slice(), "até logo".as_bytes()].concat();
        stream.write_all(&masked(8, true, &payload)).unwrap();
        assert_eq!(read_frame(&mut stream), (8, payload));
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    }
}
#[test]
fn invalid_opening_is_http_and_negotiation_is_explicit_all_modes() {
    for mode in modes() {
        let server = Running::new(mode, config());
        for (from, to, expected) in [
            ("Version: 13", "Version: 12", "426"),
            ("GET", "POST", "405"),
            ("GET", "HEAD", "400"),
        ] {
            let mut stream = support::connect(server.addr);
            stream
                .write_all(
                    std::str::from_utf8(OPEN)
                        .unwrap()
                        .replace(from, to)
                        .as_bytes(),
                )
                .unwrap();
            assert!(read_head(&mut stream).starts_with(&format!("HTTP/1.1 {expected}")));
        }
        let mut stream = support::connect(server.addr);
        let opening = std::str::from_utf8(OPEN).unwrap().replace("\r\n\r\n", "\r\nSec-WebSocket-Extensions: permessage-deflate\r\nSec-WebSocket-Protocol: chat, echo\r\n\r\n");
        stream.write_all(opening.as_bytes()).unwrap();
        let head = read_head(&mut stream);
        assert!(head.starts_with("HTTP/1.1 101"));
        assert!(!head.contains("Sec-WebSocket-Extensions"));
        assert!(!head.contains("Sec-WebSocket-Protocol"));
        close(&mut stream);
    }
}
#[test]
fn protocol_errors_send_close_all_modes() {
    let mut oversized = vec![0x82, 255];
    oversized.extend_from_slice(&((MAX_FRAME + 1) as u64).to_be_bytes());
    let cases = vec![
        (vec![0x81, 0], 1002u16),
        (vec![0xf1, 128], 1002),
        (vec![0x83, 128], 1002),
        (vec![0x09, 128], 1002),
        (vec![0x89, 254], 1002),
        (masked(0, true, b"x"), 1002),
        (masked(1, true, &[255]), 1007),
        (masked(8, true, &[0]), 1002),
        (masked(8, true, &1005u16.to_be_bytes()), 1002),
        (masked(8, true, &[3, 232, 255]), 1007),
        (oversized, 1009),
    ];
    for mode in modes() {
        let server = Running::new(mode, config());
        for (wire, code) in &cases {
            let mut stream = server.open();
            stream.write_all(wire).unwrap();
            assert_eq!(
                read_frame(&mut stream),
                (8, code.to_be_bytes().to_vec()),
                "{mode:?}"
            );
            assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        }
    }
}
#[test]
fn message_limit_and_large_echo_all_modes() {
    for mode in modes() {
        let server = Running::new(mode, config());
        let mut stream = server.open();
        let payload = vec![42; MAX_MESSAGE];
        stream.write_all(&masked(2, true, &payload)).unwrap();
        assert_eq!(read_frame(&mut stream), (2, payload.clone()));
        stream.write_all(&masked(2, false, &payload)).unwrap();
        stream.write_all(&masked(0, true, b"x")).unwrap();
        assert_eq!(read_frame(&mut stream), (8, 1009u16.to_be_bytes().to_vec()));
    }
}
#[test]
fn shutdown_sends_close_and_has_bounded_wait_all_modes() {
    for mode in modes() {
        let mut server = Running::new(mode, config());
        let mut stream = server.open();
        let start = Instant::now();
        server.shutdown.request_shutdown();
        assert_eq!(read_frame(&mut stream), (8, 1001u16.to_be_bytes().to_vec()));
        // An unresponsive peer must not prevent shutdown from completing.
        server.stop();
        assert!(start.elapsed() < Duration::from_secs(2), "{mode:?}");
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        assert_eq!(server.stats.snapshot().active, 0);
    }
}
#[test]
fn server_close_accepts_peer_reply_without_data_echo_all_modes() {
    for mode in modes() {
        let mut server = Running::new(mode, config());
        let mut stream = server.open();
        server.shutdown.request_shutdown();
        assert_eq!(read_frame(&mut stream).0, 8);
        let mut reply = masked(1, true, b"after close");
        reply.extend(masked(8, true, &1000u16.to_be_bytes()));
        stream.write_all(&reply).unwrap();
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        server.stop();
    }
}
#[test]
fn idle_and_incomplete_frame_deadlines_all_modes() {
    for mode in modes() {
        let server = Running::new(
            mode,
            Config {
                idle_timeout: Duration::from_millis(80),
                ..config()
            },
        );
        let mut stream = server.open();
        assert_eq!(read_frame(&mut stream).0, 8);
        let server = Running::new(
            mode,
            Config {
                idle_timeout: Duration::from_secs(1),
                request_timeout: Duration::from_millis(100),
                ..config()
            },
        );
        let mut stream = server.open();
        stream.write_all(&[0x81]).unwrap();
        let start = Instant::now();
        thread::sleep(Duration::from_millis(45));
        stream.write_all(&[0x80]).unwrap();
        assert_eq!(read_frame(&mut stream), (8, 1001u16.to_be_bytes().to_vec()));
        assert!(start.elapsed() < Duration::from_millis(600));
    }
}
#[test]
fn eof_during_frame_releases_connection_all_modes() {
    for mode in modes() {
        let server = Running::new(mode, config());
        let mut stream = server.open();
        stream.write_all(&[0x81, 0x85, 1]).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        health(&server);
    }
}
#[test]
fn persistent_sessions_occupy_pool_workers_and_release_them() {
    let server = Running::new(
        Execution::Pool {
            workers: 2,
            queue_capacity: 4,
        },
        config(),
    );
    let mut first = server.open();
    let mut second = server.open();
    let mut third = support::connect(server.addr);
    third
        .set_read_timeout(Some(Duration::from_millis(80)))
        .unwrap();
    third
        .write_all(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
        .unwrap();
    let error = third.read(&mut [0]).unwrap_err();
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
    close(&mut first);
    third
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert!(read_head(&mut third).starts_with("HTTP/1.1 200"));
    close(&mut second);
}
#[test]
fn slow_reader_keeps_health_responsive_and_times_out_readiness_modes() {
    for mode in modes()
        .into_iter()
        .filter(|mode| matches!(mode, Execution::Tokio { .. } | Execution::Epoll { .. }))
    {
        let server = Running::new(
            mode,
            Config {
                write_timeout: Duration::from_millis(150),
                ..config()
            },
        );
        let stream = server.open();
        let mut writer = stream.try_clone().unwrap();
        writer
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let sender = thread::spawn(move || {
            let frame = masked(2, true, &vec![77; MAX_MESSAGE]);
            for _ in 0..32 {
                if writer.write_all(&frame).is_err() {
                    return;
                }
            }
        });
        thread::sleep(Duration::from_millis(80));
        health(&server);
        sender.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.stats.snapshot().active != 0 {
            assert!(
                Instant::now() < deadline,
                "slow session remained active in {mode:?}"
            );
            thread::sleep(Duration::from_millis(5));
        }
        drop(stream);
    }
}
