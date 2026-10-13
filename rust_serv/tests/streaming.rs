mod support;
use bytes::Bytes;
use rust_serv::{
    connection::Config,
    http::{
        body::{BLOCK_SIZE, Body, Chunk},
        parser::{MAX_BODY, ParseError, RequestParser, parse_request},
        response::{Response, Status},
    },
    routes,
    server::{Execution, Server, ShutdownHandle},
    static_files::StaticFiles,
    stats::Stats,
};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use support::{ResponseReader, connect};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rust-serv-stream-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn sparse(&self, size: u64) -> File {
        let file = File::create(self.0.join("big.bin")).unwrap();
        file.set_len(size).unwrap();
        File::open(self.0.join("big.bin")).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Running {
    addr: SocketAddr,
    shutdown: ShutdownHandle,
    stats: Arc<Stats>,
    join: Option<thread::JoinHandle<io::Result<()>>>,
}
impl Running {
    fn new(mode: Execution, root: &Fixture) -> Self {
        let server = Server::bind_with_config(
            "127.0.0.1:0",
            routes::static_router(StaticFiles::new(&root.0).unwrap()),
            Config {
                idle_timeout: Duration::from_secs(2),
                request_timeout: Duration::from_secs(2),
                write_timeout: Duration::from_secs(2),
                max_requests: 100,
            },
        )
        .unwrap()
        .with_execution(mode)
        .unwrap();
        let addr = server.local_addr().unwrap();
        let shutdown = server.shutdown_handle();
        let stats = server.stats();
        Self {
            addr,
            shutdown,
            stats,
            join: Some(thread::spawn(move || server.run())),
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown.request_shutdown();
        if let Some(join) = self.join.take() {
            join.join().unwrap().unwrap();
        }
    }
}
fn modes() -> Vec<Execution> {
    let mut modes = vec![
        Execution::ThreadPerConnection,
        Execution::Pool {
            workers: 4,
            queue_capacity: 16,
        },
        Execution::Tokio {
            max_connections: 64,
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
fn chunked(payload: &[u8]) -> Vec<u8> {
    let mut bytes =
        b"POST /echo HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    bytes.extend_from_slice(payload);
    bytes
}
fn decoded(payload: &[u8]) -> Vec<u8> {
    parse_request(&chunked(payload)).unwrap().unwrap().0.body
}

#[test]
fn chunked_binary_extensions_and_metadata_trailers() {
    assert_eq!(
        decoded(
            b"3;foo=\"a;b\\\"c\"; bar=yes\r\na\0b\r\n2\r\ncd\r\n0\r\nX-Checksum: ignored\r\n\r\n"
        ),
        b"a\0bcd"
    );
    assert_eq!(decoded(b"0\r\n\r\n"), b"");
    assert_eq!(decoded(b"A\r\n0123456789\r\n0\r\n\r\n"), b"0123456789");
}
#[test]
fn every_fragment_boundary_and_pipelining() {
    let wire = chunked(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n");
    for end in 0..wire.len() {
        assert!(parse_request(&wire[..end]).unwrap().is_none(), "{end}");
    }
    let mut parser = RequestParser::default();
    for end in 0..wire.len() {
        assert!(parser.parse(&wire[..end]).unwrap().is_none(), "{end}");
    }
    let mut pipeline = wire.clone();
    pipeline.extend_from_slice(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    let (request, n) = parser.parse(&pipeline).unwrap().unwrap();
    assert_eq!(request.body, b"Wikipedia");
    assert_eq!(n, wire.len());
    assert_eq!(
        parser.parse(&pipeline[n..]).unwrap().unwrap().0.path(),
        "/health"
    );
}
#[test]
fn malformed_chunks_are_rejected() {
    for data in [
        b"g\r\n".as_slice(),
        b"+1\r\n",
        b" 1\r\n",
        b"1 \r\n",
        b"1\r\nxXX",
        b"1;\r\n",
        b"1;foo=\r\n",
        b"1;x=\"unterminated\r\n",
        b"0\r\nContent-Length: 3\r\n\r\n",
        b"0\r\nHost: evil\r\n\r\n",
    ] {
        assert!(parse_request(&chunked(data)).is_err(), "{data:?}");
    }
}
#[test]
fn request_limits_cover_decoded_body_and_framing() {
    let oversized = format!("{:x}\r\n", MAX_BODY + 1);
    assert_eq!(
        parse_request(&chunked(oversized.as_bytes())).unwrap_err(),
        ParseError::BodyTooLarge
    );
    assert_eq!(
        parse_request(&chunked(b"ffffffffffffffffffffffffffffffff\r\n")).unwrap_err(),
        ParseError::BodyTooLarge
    );
    let mut payload = format!("{:x}\r\n", MAX_BODY).into_bytes();
    payload.resize(payload.len() + MAX_BODY, b'a');
    payload.extend_from_slice(b"\r\n1\r\n");
    assert_eq!(
        parse_request(&chunked(&payload)).unwrap_err(),
        ParseError::BodyTooLarge
    );
    let long = format!("1;foo={}\r\n", "x".repeat(8192));
    assert_eq!(
        parse_request(&chunked(long.as_bytes())).unwrap_err(),
        ParseError::HeadersTooLarge
    );
}
#[test]
fn ambiguous_request_framing_is_rejected_before_body() {
    for fields in [
        "Transfer-Encoding: chunked\r\nContent-Length: 0",
        "Content-Length: abc\r\nTransfer-Encoding: chunked",
        "Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked",
    ] {
        let wire = format!("POST /echo HTTP/1.1\r\nHost: x\r\n{fields}\r\n\r\n");
        assert_eq!(
            parse_request(wire.as_bytes()).unwrap_err(),
            ParseError::AmbiguousFraming
        );
    }
    assert_eq!(
        parse_request(b"POST /echo HTTP/1.0\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap_err(),
        ParseError::AmbiguousFraming
    );
}
#[test]
fn byte_chunks_share_the_original_allocation() {
    let bytes = Bytes::from(vec![7; BLOCK_SIZE * 2 + 1]);
    let pointer = bytes.as_ptr();
    let mut cursor = Body::Bytes(bytes).cursor().unwrap();
    let Chunk::Data(first) = cursor.next_chunk().unwrap() else {
        panic!()
    };
    assert_eq!(first.as_ptr(), pointer);
    assert_eq!(first.len(), BLOCK_SIZE);
    let Chunk::Data(second) = cursor.next_chunk().unwrap() else {
        panic!()
    };
    assert_eq!(second.as_ptr(), pointer.wrapping_add(BLOCK_SIZE));
}
#[test]
fn file_serializer_uses_bounded_blocks_and_can_be_repeated() {
    struct Sink {
        count: u64,
        largest: usize,
    }
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            assert!(b.len() <= BLOCK_SIZE);
            self.count += b.len() as u64;
            self.largest = self.largest.max(b.len());
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let root = Fixture::new();
    let len = 20 * 1024 * 1024;
    let response = Response::new(Status::Ok).body(Body::File {
        file: root.sparse(len),
        offset: 0,
        len,
    });
    let mut head = Vec::new();
    response.write_head_to(&mut head).unwrap();
    for _ in 0..2 {
        let mut sink = Sink {
            count: 0,
            largest: 0,
        };
        response.write_to(&mut sink).unwrap();
        assert_eq!(sink.count, len + head.len() as u64);
        assert_eq!(sink.largest, BLOCK_SIZE);
    }
}
#[test]
fn file_shrink_is_an_error_and_growth_does_not_change_declared_length() {
    let root = Fixture::new();
    let file = root.sparse(2);
    let response = Response::new(Status::Ok).body(Body::File {
        file,
        len: 4,
        offset: 0,
    });
    assert_eq!(
        response.write_to(&mut Vec::new()).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let response = Response::new(Status::Ok).body(Body::File {
        file: root.sparse(8),
        offset: 0,
        len: 2,
    });
    let mut wire = Vec::new();
    response.write_to(&mut wire).unwrap();
    assert!(wire.ends_with(b"\r\n\r\n\0\0"));
}
#[test]
fn unknown_length_has_one_framing_and_terminal_chunk() {
    let response = Response::new(Status::Ok)
        .header("Content-Length", "999")
        .header("Transfer-Encoding", "gzip")
        .body(Body::Stream {
            interval: Duration::ZERO,
            events: 1,
        });
    let mut wire = Vec::new();
    response.write_to(&mut wire).unwrap();
    let text = String::from_utf8(wire).unwrap();
    assert!(!text.contains("Content-Length"));
    assert!(text.contains("Transfer-Encoding: chunked\r\n"));
    assert!(text.ends_with("0\r\n\r\n"));
    assert!(text.contains("id: 1\ndata: event 1\n\n"));
    let mut head = Vec::new();
    response.write_head_to(&mut head).unwrap();
    assert!(head.ends_with(b"\r\n\r\n"));
    assert!(!head.ends_with(b"0\r\n\r\n"));
}
#[test]
fn statuses_without_body_also_suppress_chunked() {
    for status in [Status::NoContent, Status::NotModified] {
        let response = Response::new(status).body(Body::Stream {
            interval: Duration::ZERO,
            events: 10,
        });
        let mut wire = Vec::new();
        response.write_to(&mut wire).unwrap();
        let text = String::from_utf8(wire).unwrap();
        assert!(!text.contains("Transfer-Encoding"));
        assert!(!text.contains("Content-Length"));
        assert!(!text.contains("data:"));
    }
}
#[test]
fn all_transports_decode_chunked_and_keep_pipeline_aligned() {
    for mode in modes() {
        let root = Fixture::new();
        let server = Running::new(mode, &root);
        let mut reader = ResponseReader::new(connect(server.addr));
        let mut wire = chunked(b"3\r\na\0b\r\n0\r\n\r\n");
        wire.extend_from_slice(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        reader.send(&wire);
        assert_eq!(reader.read_response(false).body, b"a\0b", "{mode:?}");
        assert_eq!(reader.read_response(false).body, b"ok\n");
        reader.assert_eof();
    }
}
#[test]
fn all_transports_close_smuggling_requests_without_dispatching_pipeline() {
    for mode in modes() {
        let root = Fixture::new();
        let server = Running::new(mode, &root);
        let mut reader = ResponseReader::new(connect(server.addr));
        reader.send(b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(
            reader.read_response(false).status,
            "HTTP/1.1 400 Bad Request"
        );
        reader.assert_eof();
        assert_eq!(server.stats.snapshot().requests, 0);
    }
}
#[test]
fn all_transports_stream_files_and_head_without_losing_alignment() {
    for mode in modes() {
        let root = Fixture::new();
        let bytes: Vec<u8> = (0..8 * 1024 * 1024).map(|n| (n % 251) as u8).collect();
        fs::write(root.0.join("big.bin"), &bytes).unwrap();
        let server = Running::new(mode, &root);
        let mut reader = ResponseReader::new(connect(server.addr));
        reader.send(b"HEAD /big.bin HTTP/1.1\r\nHost: x\r\n\r\nGET /big.bin HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        let head = reader.read_response(true);
        assert_eq!(head.header("Content-Length"), bytes.len().to_string());
        assert!(head.body.is_empty());
        assert_eq!(reader.read_response(false).body, bytes);
        reader.assert_eof();
    }
}
#[test]
fn all_transports_send_sse_then_a_second_response() {
    for mode in modes() {
        let root = Fixture::new();
        let server = Running::new(mode, &root);
        let mut reader = ResponseReader::new(connect(server.addr));
        reader.send(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        let response = reader.read_response(false);
        assert_eq!(response.header("Transfer-Encoding"), "chunked");
        assert!(response.optional_header("Content-Length").is_none());
        assert!(response.body.ends_with(b"data: event 10\n\n"));
        assert_eq!(reader.read_response(false).body, b"ok\n");
        reader.assert_eof();
    }
}
#[test]
fn tokio_sse_arrives_periodically_and_disconnect_releases_task() {
    let root = Fixture::new();
    let server = Running::new(
        Execution::Tokio {
            max_connections: 64,
        },
        &root,
    );
    let mut stream = connect(server.addr);
    stream
        .write_all(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let start = Instant::now();
    let mut received = Vec::new();
    let mut buffer = [0; 1024];
    while !received.windows(13).any(|w| w == b"data: event 1") {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        received.extend_from_slice(&buffer[..n]);
    }
    assert!(start.elapsed() < Duration::from_millis(200));
    assert!(!received.windows(13).any(|w| w == b"data: event 2"));
    while !received.windows(13).any(|w| w == b"data: event 2") {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0);
        received.extend_from_slice(&buffer[..n]);
    }
    assert!(start.elapsed() >= Duration::from_millis(200));
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(2);
    while server.stats.snapshot().active != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.stats.snapshot().active, 0);
}
#[test]
fn tokio_shutdown_finishes_stream_with_zero_chunk() {
    let root = Fixture::new();
    let server = Running::new(
        Execution::Tokio {
            max_connections: 64,
        },
        &root,
    );
    let mut reader = ResponseReader::new(connect(server.addr));
    reader.send(b"GET /stream HTTP/1.1\r\nHost: x\r\n\r\n");
    let deadline = Instant::now() + Duration::from_secs(1);
    while server.stats.snapshot().requests == 0 {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    thread::sleep(Duration::from_millis(30));
    let start = Instant::now();
    server.shutdown.request_shutdown();
    let response = reader.read_response(false);
    assert!(response.body.len() < 250);
    assert!(start.elapsed() < Duration::from_secs(1));
    reader.assert_eof();
}
#[test]
fn http10_unknown_length_route_is_explicitly_rejected() {
    for mode in modes() {
        let root = Fixture::new();
        let server = Running::new(mode, &root);
        let mut reader = ResponseReader::new(connect(server.addr));
        reader.send(b"GET /stream HTTP/1.0\r\n\r\n");
        let response = reader.read_response(false);
        assert_eq!(response.status, "HTTP/1.1 505 HTTP Version Not Supported");
        assert!(response.optional_header("Transfer-Encoding").is_none());
        reader.assert_eof();
    }
}
