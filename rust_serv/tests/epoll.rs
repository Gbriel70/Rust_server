#![cfg(target_os = "linux")]
mod support;
use rust_serv::{
    connection::Config,
    epoll::Epoll,
    http::{
        request::Method,
        response::{Response, Status},
    },
    router::Router,
    routes,
    server::{Execution, Server, ShutdownHandle},
    stats::Stats,
};
use std::{
    io::Read,
    net::{Shutdown, SocketAddr, TcpStream},
    os::fd::AsFd,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use support::{ResponseReader, connect};
struct Running {
    addr: SocketAddr,
    stop: ShutdownHandle,
    stats: Arc<Stats>,
    thread: Option<JoinHandle<()>>,
}
impl Running {
    fn new(edge: bool, router: Router, config: Config) -> Self {
        let server = Server::bind_with_config("127.0.0.1:0", router, config)
            .unwrap()
            .with_execution(Execution::Epoll {
                edge_triggered: edge,
            })
            .unwrap();
        let addr = server.local_addr().unwrap();
        let stop = server.shutdown_handle();
        let stats = server.stats();
        let thread = Some(thread::spawn(move || server.run().unwrap()));
        Self {
            addr,
            stop,
            stats,
            thread,
        }
    }
    fn standard(edge: bool) -> Self {
        Self::new(
            edge,
            routes::default_router(),
            Config {
                idle_timeout: Duration::from_secs(3),
                request_timeout: Duration::from_secs(3),
                write_timeout: Duration::from_secs(3),
                max_requests: 2000,
            },
        )
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.request_shutdown();
        if let Some(t) = self.thread.take() {
            t.join().unwrap();
        }
    }
}
fn request(reader: &mut ResponseReader, path: &str, close: bool) {
    reader.send(
        format!(
            "GET {path} HTTP/1.1\r\nHost: x\r\n{}\r\n",
            if close { "Connection: close\r\n" } else { "" }
        )
        .as_bytes(),
    );
}

#[test]
fn wrapper_add_modify_delete_and_timeout() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let epoll = Epoll::new().unwrap();
    epoll
        .add(listener.as_fd(), 42, libc::EPOLLIN as u32)
        .unwrap();
    assert!(epoll.wait(1).unwrap().is_empty());
    let _client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let events = epoll.wait(100).unwrap();
    assert_eq!(events[0].token, 42);
    assert_ne!(events[0].flags & libc::EPOLLIN as u32, 0);
    epoll
        .modify(listener.as_fd(), 43, libc::EPOLLIN as u32)
        .unwrap();
    assert_eq!(epoll.wait(100).unwrap()[0].token, 43);
    epoll.delete(listener.as_fd()).unwrap();
    assert!(epoll.wait(1).unwrap().is_empty());
    assert!(epoll.delete(listener.as_fd()).is_err());
}
#[test]
fn incremental_binary_pipeline_and_head_both_triggers() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        let mut r = ResponseReader::new(connect(server.addr));
        r.send(b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 256\r");
        thread::sleep(Duration::from_millis(25));
        r.send(b"\n\r\n");
        let body: Vec<u8> = (0..=255).collect();
        r.send(&body[..100]);
        thread::sleep(Duration::from_millis(25));
        r.send(&body[100..]);
        r.send(b"HEAD /health HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(r.read_response(false).body, body);
        assert_eq!(r.read_response(true).header("Content-Length"), "3");
        assert_eq!(r.read_response(false).body, b"ok\n");
    }
}
#[test]
fn half_close_keeps_complete_requests_both_triggers() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        let mut r = ResponseReader::new(connect(server.addr));
        request(&mut r, "/health", false);
        r.stream.shutdown(Shutdown::Write).unwrap();
        assert_eq!(r.read_response(false).body, b"ok\n");
    }
}
#[test]
fn incomplete_eof_and_bad_request_both_triggers() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        for bytes in [
            b"GET /health HTTP/1.1\r\nHost:".as_slice(),
            b"BOGUS\r\n\r\n",
        ] {
            let mut r = ResponseReader::new(connect(server.addr));
            r.send(bytes);
            r.stream.shutdown(Shutdown::Write).unwrap();
            assert!(r.read_response(false).status.contains("400"));
        }
    }
}
#[test]
fn idle_and_total_request_deadlines_both_triggers() {
    for edge in [false, true] {
        let server = Running::new(
            edge,
            routes::default_router(),
            Config {
                idle_timeout: Duration::from_millis(70),
                request_timeout: Duration::from_millis(100),
                write_timeout: Duration::from_secs(1),
                ..Config::default()
            },
        );
        let mut idle = connect(server.addr);
        let mut byte = [0];
        assert_eq!(idle.read(&mut byte).unwrap(), 0);
        let mut r = ResponseReader::new(connect(server.addr));
        r.send(b"GET /health HTTP/1.1\r\nHost:");
        assert!(r.read_response(false).status.contains("408"));
    }
}
#[test]
fn handler_panic_does_not_kill_event_loop() {
    for edge in [false, true] {
        let router =
            routes::default_router().route(Method::Get, "/panic", |_| panic!("intentional"));
        let server = Running::new(edge, router, Config::default());
        let mut r = ResponseReader::new(connect(server.addr));
        request(&mut r, "/panic", false);
        assert!(r.read_response(false).status.contains("500"));
        request(&mut r, "/health", true);
        assert_eq!(r.read_response(false).body, b"ok\n");
        assert_eq!(server.stats.snapshot().panics, 1);
    }
}
#[test]
fn thousand_idle_connections_do_not_occupy_workers() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        let idle: Vec<_> = (0..1000).map(|_| connect(server.addr)).collect();
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.stats.snapshot().active < 1000 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(2));
        }
        let start = Instant::now();
        let mut r = ResponseReader::new(connect(server.addr));
        request(&mut r, "/health", true);
        assert_eq!(r.read_response(false).body, b"ok\n");
        assert!(
            start.elapsed() < Duration::from_millis(50),
            "latency {:?}",
            start.elapsed()
        );
        drop(idle);
    }
}
#[test]
fn ten_thousand_concurrent_requests_count_exactly() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        let workers: Vec<_> = (0..20)
            .map(|_| {
                let addr = server.addr;
                thread::spawn(move || {
                    let mut r = ResponseReader::new(connect(addr));
                    for n in 0..500 {
                        request(&mut r, "/counter", n == 499);
                        assert!(r.read_response(false).status.contains("200"));
                    }
                })
            })
            .collect();
        for t in workers {
            t.join().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.stats.snapshot().responses != 10000 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(server.stats.snapshot().requests, 10000);
        let mut r = ResponseReader::new(connect(server.addr));
        request(&mut r, "/counter", true);
        assert_eq!(r.read_response(false).body, b"10001");
    }
}
#[test]
fn slow_reader_partial_writes_do_not_block_health() {
    for edge in [false, true] {
        let bytes: Vec<u8> = (0..8 * 1024 * 1024).map(|n| (n % 251) as u8).collect();
        let expected = bytes.clone();
        let router = routes::default_router().route(Method::Get, "/large", move |_| {
            Response::new(Status::Ok).body(bytes.clone())
        });
        let server = Running::new(
            edge,
            router,
            Config {
                write_timeout: Duration::from_secs(5),
                ..Config::default()
            },
        );
        let mut slow = ResponseReader::new(connect(server.addr));
        request(&mut slow, "/large", true);
        thread::sleep(Duration::from_millis(60));
        let mut fast = ResponseReader::new(connect(server.addr));
        let start = Instant::now();
        request(&mut fast, "/health", true);
        assert_eq!(fast.read_response(false).body, b"ok\n");
        assert!(start.elapsed() < Duration::from_millis(50));
        assert_eq!(slow.read_response(false).body, expected);
    }
}
#[test]
fn shutdown_finishes_started_and_buffered_requests() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        let mut r = ResponseReader::new(connect(server.addr));
        r.send(b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\n\r\na");
        thread::sleep(Duration::from_millis(30));
        server.stop.request_shutdown();
        r.send(b"bcd");
        assert_eq!(r.read_response(false).body, b"abcd");
        assert_eq!(r.stream.read(&mut [0]).unwrap(), 0);
    }
}
#[test]
fn fairness_queue_resumes_edge_pipeline_without_new_edge() {
    let server = Running::standard(true);
    let mut r = ResponseReader::new(connect(server.addr));
    let mut pipeline = Vec::new();
    for n in 0..300 {
        pipeline.extend_from_slice(
            format!(
                "GET /health HTTP/1.1\r\nHost: x\r\n{}\r\n",
                if n == 299 {
                    "Connection: close\r\n"
                } else {
                    ""
                }
            )
            .as_bytes(),
        );
    }
    r.send(&pipeline);
    for _ in 0..300 {
        assert_eq!(r.read_response(false).body, b"ok\n");
    }
}
#[test]
fn repeated_fd_reuse_never_mixes_clients() {
    for edge in [false, true] {
        let server = Running::standard(edge);
        for _ in 0..150 {
            let mut r = ResponseReader::new(connect(server.addr));
            request(&mut r, "/health", true);
            assert_eq!(r.read_response(false).body, b"ok\n");
        }
    }
}
#[test]
fn cli_selects_level_and_edge() {
    use rust_serv::cli::Options;
    for (args, edge) in [
        (vec!["--mode", "epoll"], false),
        (vec!["--mode", "epoll", "--trigger", "edge"], true),
    ] {
        assert_eq!(
            Options::parse(args.into_iter().map(str::to_owned))
                .unwrap()
                .execution,
            Execution::Epoll {
                edge_triggered: edge
            }
        );
    }
    assert!(
        Options::parse(["--mode", "epoll", "--trigger", "invalid"].map(str::to_owned)).is_err()
    );
}

#[test]
fn no_body_statuses_keep_pipeline_aligned() {
    for edge in [false, true] {
        let router = routes::default_router()
            .route(Method::Get, "/cached", |_| {
                Response::new(Status::NotModified).body("must not be sent")
            })
            .route(Method::Get, "/empty", |_| {
                Response::new(Status::NoContent).body("must not be sent")
            });
        let server = Running::new(edge, router, Config::default());
        let mut r = ResponseReader::new(connect(server.addr));
        r.send(b"GET /cached HTTP/1.1\r\nHost: x\r\n\r\nGET /empty HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        for code in ["304", "204"] {
            let response = r.read_response(false);
            assert!(response.status.contains(code));
            assert!(response.optional_header("Content-Length").is_none());
            assert!(response.body.is_empty());
        }
        assert_eq!(r.read_response(false).body, b"ok\n");
    }
}
#[test]
fn http10_keep_alive_and_max_requests() {
    for edge in [false, true] {
        let server = Running::new(
            edge,
            routes::default_router(),
            Config {
                max_requests: 2,
                ..Config::default()
            },
        );
        let mut r = ResponseReader::new(connect(server.addr));
        r.send(b"GET /health HTTP/1.0\r\nConnection: keep-alive\r\n\r\n");
        assert_eq!(r.read_response(false).header("Connection"), "keep-alive");
        request(&mut r, "/health", false);
        assert_eq!(r.read_response(false).header("Connection"), "close");
        assert_eq!(r.stream.read(&mut [0]).unwrap(), 0);
    }
}
#[test]
fn pending_write_deadline_releases_slow_socket() {
    for edge in [false, true] {
        let router = routes::default_router().route(Method::Get, "/large", |_| {
            Response::new(Status::Ok).body(vec![7; 8 * 1024 * 1024])
        });
        let server = Running::new(
            edge,
            router,
            Config {
                write_timeout: Duration::from_millis(100),
                ..Config::default()
            },
        );
        let mut slow = ResponseReader::new(connect(server.addr));
        request(&mut slow, "/large", true);
        let deadline = Instant::now() + Duration::from_secs(2);
        while server.stats.snapshot().completed == 0 {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(server.stats.snapshot().responses, 0);
        let mut fast = ResponseReader::new(connect(server.addr));
        request(&mut fast, "/health", true);
        assert_eq!(fast.read_response(false).body, b"ok\n");
    }
}

#[test]
fn descriptor_exhaustion_recovers_after_listener_backoff() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Child, Command, Stdio},
    };
    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if let Some(stdin) = self.0.stdin.as_mut() {
                let _ = stdin.write_all(b"\n");
            }
            let deadline = Instant::now() + Duration::from_secs(4);
            while self.0.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for trigger in ["level", "edge"] {
        // Limit only the child: no unsafe pre_exec and no impact on other tests.
        let mut child = ChildGuard(
            Command::new("bash")
                .args(["-c", "ulimit -n 64; exec \"$@\"", "epoll-emfile-test"])
                .arg(env!("CARGO_BIN_EXE_rust_serv"))
                .args([
                    "--mode",
                    "epoll",
                    "--trigger",
                    trigger,
                    "--addr",
                    "127.0.0.1:0",
                    "--root",
                ])
                .arg(format!("{}/public", env!("CARGO_MANIFEST_DIR")))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        let mut line = String::new();
        BufReader::new(child.0.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let addr: SocketAddr = line
            .strip_prefix("Listening on ")
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let mut clients: Vec<_> = (0..90).map(|_| connect(addr)).collect();
        thread::sleep(Duration::from_millis(180));
        let mut existing = ResponseReader::new(clients.remove(0));
        request(&mut existing, "/stats", true);
        let response = existing.read_response(false);
        assert!(response.status.contains("200"));
        let json = String::from_utf8(response.body).unwrap();
        let errors: usize = json
            .split("\"errors\":")
            .nth(1)
            .unwrap()
            .split(',')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(errors > 0, "{json}");
        drop(existing);
        drop(clients);
        thread::sleep(Duration::from_millis(150));
        let mut recovered = ResponseReader::new(connect(addr));
        request(&mut recovered, "/health", true);
        assert_eq!(recovered.read_response(false).body, b"ok\n");
    }
}
