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
    net::SocketAddr,
    sync::{Arc, Condvar, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct Running {
    addr: SocketAddr,
    shutdown: ShutdownHandle,
    stats: Arc<Stats>,
    join: Option<JoinHandle<io::Result<()>>>,
}
impl Running {
    fn new(execution: Execution, router: Router) -> Self {
        let config = Config {
            idle_timeout: Duration::from_secs(2),
            request_timeout: Duration::from_secs(3),
            write_timeout: Duration::from_secs(3),
            max_requests: 100,
        };
        let server = Server::bind_with_config("127.0.0.1:0", router, config)
            .unwrap()
            .with_execution(execution)
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
    fn stop(&mut self) {
        self.shutdown.request_shutdown();
        if let Some(join) = self.join.take() {
            join.join().unwrap().unwrap();
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.shutdown.request_shutdown();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
fn pool(n: usize) -> Execution {
    Execution::Pool {
        workers: n,
        queue_capacity: 256,
    }
}
fn until(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < end, "condition not reached");
        thread::sleep(Duration::from_millis(1));
    }
}
fn get(addr: SocketAddr, path: &str) -> support::RawResponse {
    support::send_raw(
        addr,
        format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes(),
    )
}
#[test]
fn router_and_server_are_send_sync() {
    fn require<T: Send + Sync>() {}
    require::<Router>();
    require::<Server>();
}
#[test]
fn one_hundred_idle_connections_do_not_block_thread_mode() {
    let mut server = Running::new(Execution::ThreadPerConnection, routes::default_router());
    let idle = (0..100)
        .map(|_| support::connect(server.addr))
        .collect::<Vec<_>>();
    until(|| server.stats.snapshot().active == 100);
    let start = Instant::now();
    let response = get(server.addr, "/health");
    let elapsed = start.elapsed();
    assert_eq!(response.body, b"ok\n");
    eprintln!("100 idle sockets + GET /health: {elapsed:?}");
    // Environment-dependent performance is also recorded by the external curl experiment.
    assert!(
        elapsed < Duration::from_millis(50),
        "latency target missed: {elapsed:?}"
    );
    drop(idle);
    server.stop();
    assert_eq!(server.stats.snapshot().active, 0);
}
#[test]
fn four_workers_saturate_with_five_idle_keep_alive_connections() {
    let mut server = Running::new(pool(4), routes::default_router());
    let mut held = Vec::new();
    for _ in 0..4 {
        let mut reader = support::ResponseReader::new(support::connect(server.addr));
        reader.send(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(reader.read_response(false).body, b"ok\n");
        held.push(reader);
    }
    until(|| server.stats.snapshot().active == 4);
    let mut fifth = support::ResponseReader::new(support::connect(server.addr));
    fifth.send(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    until(|| server.stats.snapshot().queued == 1);
    fifth
        .stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    let error = fifth.stream.read(&mut [0; 1]).unwrap_err();
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
    drop(held);
    fifth
        .stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    assert_eq!(fifth.read_response(false).body, b"ok\n");
    fifth.assert_eof();
    server.stop();
}
fn ten_thousand(execution: Execution) {
    let mut server = Running::new(execution, routes::default_router());
    let workers = (0..100)
        .map(|_| {
            let addr = server.addr;
            thread::spawn(move || {
                let stream = support::connect(addr);
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let mut reader = support::ResponseReader::new(stream);
                let mut numbers = Vec::new();
                for request in 0..100 {
                    reader.send(if request == 99 {
                        b"GET /counter HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
                    } else {
                        b"GET /counter HTTP/1.1\r\nHost: x\r\n\r\n"
                    });
                    let response = reader.read_response(false);
                    assert_eq!(response.status, "HTTP/1.1 200 OK");
                    numbers.push(
                        String::from_utf8(response.body)
                            .unwrap()
                            .parse::<usize>()
                            .unwrap(),
                    );
                }
                reader.assert_eof();
                numbers
            })
        })
        .collect::<Vec<_>>();
    let mut numbers = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    numbers.sort_unstable();
    assert_eq!(numbers, (1..=10000).collect::<Vec<_>>());
    until(|| server.stats.snapshot().responses == 10000);
    assert_eq!(server.stats.snapshot().requests, 10000);
    let stats = get(server.addr, "/stats");
    assert_eq!(stats.header("Content-Type"), "application/json");
    let json = String::from_utf8(stats.body).unwrap();
    assert!(json.contains("\"requests\":10001"), "{json}");
    server.stop();
    let s = server.stats.snapshot();
    assert_eq!(
        (
            s.requests,
            s.responses,
            s.active,
            s.queued,
            s.rejected,
            s.panics
        ),
        (10001, 10001, 0, 0, 0, 0)
    );
    assert_eq!(s.accepted, s.completed);
}
#[test]
fn ten_thousand_concurrent_requests_thread_mode() {
    ten_thousand(Execution::ThreadPerConnection);
}
#[test]
fn ten_thousand_concurrent_requests_pool_mode() {
    ten_thousand(pool(16));
}

fn graceful(execution: Execution) {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let router = Router::new().route(Method::Get, "/slow", move |_| {
        started_tx.send(()).unwrap();
        let (lock, wake) = &*worker_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
        Response::text(Status::Ok, "completed in flight")
    });
    let mut server = Running::new(execution, router);
    let mut reader = support::ResponseReader::new(support::connect(server.addr));
    reader.send(b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\n");
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    server.shutdown.request_shutdown();
    thread::sleep(Duration::from_millis(20));
    let prematurely_finished = server.join.as_ref().unwrap().is_finished();
    let (lock, wake) = &*gate;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    let response = reader.read_response(false);
    reader.assert_eof();
    server.stop();
    assert!(!prematurely_finished);
    assert_eq!(response.body, b"completed in flight");
    assert_eq!(response.header("Connection"), "close");
    assert_eq!(
        (
            server.stats.snapshot().requests,
            server.stats.snapshot().responses
        ),
        (1, 1)
    );
}
#[test]
fn graceful_shutdown_preserves_inflight_thread_request() {
    graceful(Execution::ThreadPerConnection);
}
#[test]
fn graceful_shutdown_preserves_inflight_pool_request() {
    graceful(pool(1));
}
#[test]
fn shutdown_drains_connections_already_queued() {
    let mut server = Running::new(pool(1), routes::default_router());
    let mut first = support::ResponseReader::new(support::connect(server.addr));
    first.send(b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    first.read_response(false);
    let mut second = support::ResponseReader::new(support::connect(server.addr));
    second.send(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    until(|| server.stats.snapshot().queued == 1);
    server.shutdown.request_shutdown();
    drop(first);
    assert_eq!(second.read_response(false).body, b"ok\n");
    second.assert_eof();
    server.stop();
    assert_eq!(server.stats.snapshot().responses, 2);
}
#[test]
fn handler_panic_does_not_remove_pool_worker_or_leak_active_count() {
    let router = routes::default_router().route(Method::Get, "/panic", |_| {
        panic!("intentional handler panic")
    });
    let mut server = Running::new(pool(1), router);
    let mut stream = support::connect(server.addr);
    use std::io::Write;
    stream
        .write_all(b"GET /panic HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    assert_eq!(stream.read(&mut [0; 1]).unwrap(), 0);
    assert_eq!(get(server.addr, "/health").body, b"ok\n");
    server.stop();
    let s = server.stats.snapshot();
    assert_eq!(
        (s.panics, s.requests, s.responses, s.active, s.completed),
        (1, 2, 1, 0, 2)
    );
}
#[test]
fn saturated_queue_rejects_and_restores_queued_counter() {
    let mut server = Running::new(
        Execution::Pool {
            workers: 1,
            queue_capacity: 1,
        },
        routes::default_router(),
    );
    let first = support::connect(server.addr);
    until(|| server.stats.snapshot().active == 1);
    let second = support::connect(server.addr);
    until(|| server.stats.snapshot().queued == 1);
    let third = support::connect(server.addr);
    until(|| server.stats.snapshot().rejected == 1);
    let s = server.stats.snapshot();
    assert_eq!((s.active, s.queued, s.accepted), (1, 1, 3));
    drop((first, second, third));
    server.stop();
    let s = server.stats.snapshot();
    assert_eq!((s.active, s.queued, s.completed, s.rejected), (0, 0, 2, 1));
}
#[test]
fn shutdown_before_run_and_repeated_shutdown_are_safe() {
    let server = Server::bind("127.0.0.1:0", routes::default_router()).unwrap();
    let shutdown = server.shutdown_handle();
    shutdown.request_shutdown();
    shutdown.request_shutdown();
    server.run().unwrap();
    assert_eq!(
        server.run().unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(server.stats().snapshot().accepted, 0);
}
#[test]
fn empty_pool_shutdown_wakes_blocked_receivers() {
    let mut server = Running::new(pool(4), routes::default_router());
    server.stop();
    assert_eq!(server.stats.snapshot().active, 0);
}
#[test]
fn stats_head_and_wrong_method_keep_router_semantics() {
    let mut server = Running::new(pool(2), routes::default_router());
    let response = support::send_raw(
        server.addr,
        b"HEAD /stats HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert!(response.body.is_empty());
    assert!(response.header("Content-Length").parse::<usize>().unwrap() > 0);
    let response = support::send_raw(
        server.addr,
        b"POST /stats HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert_eq!(response.status, "HTTP/1.1 405 Method Not Allowed");
    assert_eq!(response.header("Allow"), "GET, HEAD");
    server.stop();
}
#[test]
fn invalid_execution_is_rejected_before_run() {
    assert!(
        Execution::Pool {
            workers: 0,
            queue_capacity: 1
        }
        .validate()
        .is_err()
    );
    assert!(
        Execution::Pool {
            workers: 1,
            queue_capacity: 0
        }
        .validate()
        .is_err()
    );
}

#[test]
fn shutdown_preserves_pipeline_already_read_into_connection_buffer() {
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let (tx, rx) = mpsc::channel();
    let router = routes::default_router().route(Method::Get, "/slow", move |_| {
        tx.send(()).unwrap();
        let (lock, wake) = &*worker_gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = wake.wait(released).unwrap();
        }
        Response::text(Status::Ok, "first")
    });
    let mut server = Running::new(pool(1), router);
    let mut reader = support::ResponseReader::new(support::connect(server.addr));
    reader.send(b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    rx.recv_timeout(Duration::from_secs(2)).unwrap();
    server.shutdown.request_shutdown();
    let (lock, wake) = &*gate;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    assert_eq!(reader.read_response(false).body, b"first");
    let last = reader.read_response(false);
    assert_eq!(last.body, b"ok\n");
    assert_eq!(last.header("Connection"), "close");
    reader.assert_eof();
    server.stop();
    assert_eq!(server.stats.snapshot().responses, 2);
}
