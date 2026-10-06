use rust_serv::connection::{Config, Connection};
use rust_serv::error::Error;
use rust_serv::http::response::{Response, Status};
use rust_serv::{routes, server::Server};
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

#[test]
fn config_defaults_and_zero_values() {
    let defaults = Config::default();
    assert_eq!(defaults.idle_timeout, Duration::from_secs(5));
    assert_eq!(defaults.request_timeout, Duration::from_secs(10));
    assert_eq!(defaults.write_timeout, Duration::from_secs(10));
    assert_eq!(defaults.max_requests, 100);
    for config in [
        Config {
            idle_timeout: Duration::ZERO,
            ..defaults
        },
        Config {
            request_timeout: Duration::ZERO,
            ..defaults
        },
        Config {
            write_timeout: Duration::ZERO,
            ..defaults
        },
        Config {
            max_requests: 0,
            ..defaults
        },
    ] {
        let error = Server::bind_with_config("127.0.0.1:0", routes::default_router(), config)
            .err()
            .unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}

#[test]
fn partial_pipeline_retains_its_first_byte_time_across_handler_delay() {
    let (mut client, server) = pair();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost:")
        .unwrap();
    let mut connection = Connection::new(
        server,
        Config {
            request_timeout: Duration::from_millis(500),
            ..Config::default()
        },
    )
    .unwrap();
    assert_eq!(connection.read_request().unwrap().unwrap().target, "/");
    thread::sleep(Duration::from_millis(650));
    let start = Instant::now();
    assert!(matches!(
        connection.read_request(),
        Err(Error::RequestTimeout)
    ));
    assert!(start.elapsed() < Duration::from_millis(300));
}

#[test]
fn complete_pipeline_is_parsed_before_socket_read_or_timeout() {
    let (mut client, server) = pair();
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\n\r\n")
        .unwrap();
    let mut connection = Connection::new(
        server,
        Config {
            request_timeout: Duration::from_millis(50),
            ..Config::default()
        },
    )
    .unwrap();
    assert_eq!(connection.read_request().unwrap().unwrap().target, "/");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        connection.read_request().unwrap().unwrap().target,
        "/health"
    );
}

#[test]
fn write_timeout_is_applied_and_bounds_a_nonreading_peer() {
    let (_client, server) = pair();
    let observer = server.try_clone().unwrap();
    let timeout = Duration::from_millis(200);
    let mut connection = Connection::new(
        server,
        Config {
            write_timeout: timeout,
            ..Config::default()
        },
    )
    .unwrap();
    assert_eq!(observer.write_timeout().unwrap(), Some(timeout));
    // Exceeds the loopback send buffer even though our normal echo is limited to 1 MiB.
    let response = Response::new(Status::Ok).body(vec![b'x'; 16 * 1024 * 1024]);
    let start = Instant::now();
    let error = connection.write_response(&response, false).unwrap_err();
    assert!(matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn lingering_drain_has_a_byte_limit() {
    let (mut client, server) = pair();
    client.write_all(&vec![b'x'; 128 * 1024]).unwrap();
    let mut connection = Connection::new(server, Config::default()).unwrap();
    let start = Instant::now();
    connection.linger_close();
    // The peer stays open. Without the byte limit this would wait the full second.
    assert!(start.elapsed() < Duration::from_millis(800));
}

#[test]
fn lingering_drain_has_a_total_time_limit() {
    let (mut client, server) = pair();
    let mut connection = Connection::new(server, Config::default()).unwrap();
    let start = Instant::now();
    let draining = thread::spawn(move || connection.linger_close());
    // A continuing trickle must not renew the one-second drain deadline.
    for _ in 0..12 {
        thread::sleep(Duration::from_millis(100));
        if client.write_all(b"x").is_err() {
            break;
        }
    }
    draining.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(2));
}
