mod support;
use rust_serv::{
    connection::Config,
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
    io::{self, Read},
    net::{Shutdown, SocketAddr},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use support::{ResponseReader, connect};
struct Running {
    addr: SocketAddr,
    shutdown: ShutdownHandle,
    stats: Arc<Stats>,
    join: Option<JoinHandle<io::Result<()>>>,
}
impl Running {
    fn new(router: Router, config: Config, limit: usize) -> Self {
        let server = Server::bind_with_config("127.0.0.1:0", router, config)
            .unwrap()
            .with_execution(Execution::Tokio {
                max_connections: limit,
            })
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
    fn standard() -> Self {
        Self::new(
            routes::default_router(),
            Config {
                idle_timeout: Duration::from_secs(30),
                request_timeout: Duration::from_secs(3),
                write_timeout: Duration::from_secs(3),
                max_requests: 1000,
            },
            12_000,
        )
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
fn request(r: &mut ResponseReader, path: &str, close: bool) {
    r.send(
        format!(
            "GET {path} HTTP/1.1\r\nHost: x\r\n{}\r\n",
            if close { "Connection: close\r\n" } else { "" }
        )
        .as_bytes(),
    );
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "condition timed out");
        thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn ten_thousand_idle_sockets_and_health() {
    let mut server = Running::standard();
    let idle: Vec<_> = (0..10_000).map(|_| connect(server.addr)).collect();
    wait(|| server.stats.snapshot().active == 10_000);
    let mut r = ResponseReader::new(connect(server.addr));
    let start = Instant::now();
    request(&mut r, "/health", true);
    assert_eq!(r.read_response(false).body, b"ok\n");
    assert!(
        start.elapsed() < Duration::from_millis(50),
        "{:?}",
        start.elapsed()
    );
    server.stop();
    assert_eq!(server.stats.snapshot().active, 0);
    drop(idle);
}
#[test]
fn semaphore_backpressure_and_shutdown_while_full() {
    let mut server = Running::new(
        routes::default_router(),
        Config {
            idle_timeout: Duration::from_secs(30),
            ..Config::default()
        },
        1,
    );
    let first = connect(server.addr);
    wait(|| server.stats.snapshot().active == 1);
    let mut second = ResponseReader::new(connect(server.addr));
    request(&mut second, "/health", true);
    second
        .stream
        .set_read_timeout(Some(Duration::from_millis(60)))
        .unwrap();
    let e = second.stream.read(&mut [0]).unwrap_err();
    assert!(matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
    assert_eq!(server.stats.snapshot().accepted, 1);
    drop(first);
    second
        .stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    assert_eq!(second.read_response(false).body, b"ok\n");
    drop(second);
    let idle = connect(server.addr);
    wait(|| server.stats.snapshot().active == 1);
    let start = Instant::now();
    server.stop();
    assert!(start.elapsed() < Duration::from_secs(1));
    drop(idle);
}
#[test]
fn fragmented_shutdown_preserves_started_and_buffered_requests() {
    let mut server = Running::standard();
    let mut r = ResponseReader::new(connect(server.addr));
    request(&mut r, "/health", false);
    assert_eq!(r.read_response(false).body, b"ok\n");
    r.send(b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\n\r\na");
    thread::sleep(Duration::from_millis(40));
    server.shutdown.request_shutdown();
    r.send(b"bcdGET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(r.read_response(false).body, b"abcd");
    assert_eq!(r.read_response(false).body, b"ok\n");
    r.assert_eof();
    drop(r);
    server.stop();
    assert_eq!(server.stats.snapshot().requests, 3);
    assert_eq!(server.stats.snapshot().responses, 3);
}
#[test]
fn blocking_handler_does_not_block_io_and_shutdown_waits_for_it() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::clone(&gate);
    let (tx, rx) = mpsc::channel();
    let router = routes::default_router().route(Method::Get, "/slow", move |_| {
        tx.send(()).unwrap();
        let (lock, cv) = &*entered;
        let mut open = lock.lock().unwrap();
        while !*open {
            open = cv.wait(open).unwrap();
        }
        Response::text(Status::Ok, "finished")
    });
    let mut server = Running::new(router, Config::default(), 20);
    let mut slow = ResponseReader::new(connect(server.addr));
    request(&mut slow, "/slow", true);
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut fast = ResponseReader::new(connect(server.addr));
    request(&mut fast, "/health", true);
    assert_eq!(fast.read_response(false).body, b"ok\n");
    drop(fast);
    server.shutdown.request_shutdown();
    assert!(!server.join.as_ref().unwrap().is_finished());
    *gate.0.lock().unwrap() = true;
    gate.1.notify_all();
    assert_eq!(slow.read_response(false).body, b"finished");
    drop(slow);
    server.stop();
    assert_eq!(server.stats.snapshot().responses, 2);
}
#[test]
fn pending_write_is_drained_on_shutdown() {
    let bytes: Vec<u8> = (0..8 * 1024 * 1024).map(|n| (n % 251) as u8).collect();
    let expected = bytes.clone();
    let (tx, rx) = mpsc::channel();
    let router = routes::default_router().route(Method::Get, "/large", move |_| {
        tx.send(()).unwrap();
        Response::new(Status::Ok).body(bytes.clone())
    });
    let mut server = Running::new(
        router,
        Config {
            write_timeout: Duration::from_secs(5),
            ..Config::default()
        },
        10,
    );
    let mut r = ResponseReader::new(connect(server.addr));
    request(&mut r, "/large", true);
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    thread::sleep(Duration::from_millis(60));
    server.shutdown.request_shutdown();
    assert_eq!(r.read_response(false).body, expected);
    drop(r);
    server.stop();
    assert_eq!(server.stats.snapshot().responses, 1);
}
#[test]
fn idle_request_and_write_timeouts() {
    let router = routes::default_router().route(Method::Get, "/large", |_| {
        Response::new(Status::Ok).body(vec![0; 8 * 1024 * 1024])
    });
    let server = Running::new(
        router,
        Config {
            idle_timeout: Duration::from_millis(70),
            request_timeout: Duration::from_millis(100),
            write_timeout: Duration::from_millis(100),
            ..Config::default()
        },
        10,
    );
    let mut idle = connect(server.addr);
    assert_eq!(idle.read(&mut [0]).unwrap(), 0);
    let mut partial = ResponseReader::new(connect(server.addr));
    partial.send(b"HEAD /health HTTP/1.1\r\nHost:");
    let response = partial.read_response(true);
    assert!(response.status.contains("408"));
    assert!(response.body.is_empty());
    drop(partial);
    let mut slow = ResponseReader::new(connect(server.addr));
    request(&mut slow, "/large", true);
    wait(|| server.stats.snapshot().errors > 0);
    assert_eq!(server.stats.snapshot().responses, 1);
}
#[test]
fn half_close_and_handler_panic() {
    let router = routes::default_router().route(Method::Get, "/panic", |_| panic!("intentional"));
    let server = Running::new(router, Config::default(), 10);
    let mut r = ResponseReader::new(connect(server.addr));
    request(&mut r, "/panic", false);
    assert!(r.read_response(false).status.contains("500"));
    request(&mut r, "/health", false);
    r.stream.shutdown(Shutdown::Write).unwrap();
    assert_eq!(r.read_response(false).body, b"ok\n");
    r.assert_eof();
    assert_eq!(server.stats.snapshot().panics, 1);
}
#[test]
fn ten_thousand_requests_have_exact_counter() {
    let server = Running::standard();
    let clients: Vec<_> = (0..20)
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
    for client in clients {
        client.join().unwrap();
    }
    wait(|| server.stats.snapshot().responses == 10_000);
    assert_eq!(server.stats.snapshot().requests, 10_000);
    let mut r = ResponseReader::new(connect(server.addr));
    request(&mut r, "/counter", true);
    assert_eq!(r.read_response(false).body, b"10001");
}
#[tokio::test]
async fn async_api_and_sync_bridge_validation() {
    let server = Server::bind("127.0.0.1:0", routes::default_router())
        .unwrap()
        .with_execution(Execution::Tokio {
            max_connections: 10,
        })
        .unwrap();
    assert_eq!(
        server.run().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
    server.shutdown_handle().request_shutdown();
    server.run_async().await.unwrap();
    assert_eq!(
        server.run_async().await.unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    let legacy = Server::bind("127.0.0.1:0", routes::default_router()).unwrap();
    assert_eq!(
        legacy.run_async().await.unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );
}
#[test]
fn cli_and_limits() {
    use rust_serv::cli::Options;
    let options =
        Options::parse(["--mode", "tokio", "--max-connections", "42"].map(str::to_owned)).unwrap();
    assert_eq!(
        options.execution,
        Execution::Tokio {
            max_connections: 42
        }
    );
    assert_eq!(
        Options::parse(["--mode", "tokio"].map(str::to_owned))
            .unwrap()
            .execution,
        Execution::Tokio {
            max_connections: 16_384
        }
    );
    for n in ["0", "-1", "bad"] {
        assert!(
            Options::parse(["--mode", "tokio", "--max-connections", n].map(str::to_owned)).is_err()
        );
    }
    assert!(
        Execution::Tokio {
            max_connections: usize::MAX
        }
        .validate()
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn ctrl_c_drains_cli_server_and_exits_successfully() {
    use std::{
        io::{BufRead, BufReader, Write},
        process::{Child, Command, Stdio},
    };
    struct ChildGuard(Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = ChildGuard(
        Command::new(env!("CARGO_BIN_EXE_rust_serv"))
            .args(["--mode", "tokio", "--addr", "127.0.0.1:0", "--root"])
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
    let addr = line
        .strip_prefix("Listening on ")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let mut r = ResponseReader::new(connect(addr));
    request(&mut r, "/health", false);
    assert_eq!(r.read_response(false).body, b"ok\n");
    // Signal delivery only after a request proves the Tokio accept loop started.
    assert!(
        Command::new("kill")
            .args(["-INT", &child.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(r.stream.read(&mut [0]).unwrap(), 0);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    // EOF/unused stdin is harmless; it was kept open to test Ctrl-C alone.
    let _ = child.0.stdin.take().unwrap().write_all(b"");
}
