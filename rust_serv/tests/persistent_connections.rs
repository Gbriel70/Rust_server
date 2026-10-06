mod support;

use rust_serv::connection::Config;
use rust_serv::http::parser::MAX_BODY;
use std::io::Write;
use std::net::Shutdown;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use support::{ResponseReader, connect, start_server_with_config, test_config};

const GET: &[u8] = b"GET /health HTTP/1.1\r\nHost: x\r\n\r\n";

fn client(config: Config) -> ResponseReader {
    ResponseReader::new(connect(start_server_with_config(config)))
}

fn assert_close(reader: &mut ResponseReader, status: &str) {
    let response = reader.read_response(false);
    assert_eq!(response.status, status);
    assert_eq!(response.header("Connection"), "close");
    reader.assert_eof();
}

fn timeout_margin(elapsed: Duration) {
    assert!(
        elapsed >= Duration::from_millis(150),
        "too early: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(2), "too late: {elapsed:?}");
}

#[test]
fn three_sequential_requests_keep_the_connection_open() {
    let mut reader = client(test_config());
    for _ in 0..3 {
        reader.send(GET);
        let response = reader.read_response(false);
        assert_eq!(response.body, b"ok\n");
        assert_eq!(response.optional_header("Connection"), None);
    }
    reader.send(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert_close(&mut reader, "HTTP/1.1 200 OK");
}

#[test]
fn two_requests_in_one_write_return_in_order() {
    let mut reader = client(test_config());
    reader.send(b"GET / HTTP/1.1\r\nHost: x\r\n\r\nGET /health HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_eq!(reader.read_response(false).body, b"hello\n");
    assert_eq!(reader.read_response(false).body, b"ok\n");
}

#[test]
fn third_pipelined_request_is_split_between_writes() {
    let mut reader = client(test_config());
    let mut bytes = GET.repeat(2);
    bytes.extend_from_slice(&GET[..15]);
    reader.send(&bytes);
    for _ in 0..2 {
        assert_eq!(reader.read_response(false).body, b"ok\n");
    }
    thread::sleep(Duration::from_millis(50));
    reader.send(&GET[15..]);
    assert_eq!(reader.read_response(false).body, b"ok\n");
}

#[test]
fn explicit_close_ignores_later_pipelined_requests() {
    let mut reader = client(test_config());
    reader.send(b"GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\nGET / HTTP/1.1\r\nHost: x\r\n\r\n");
    assert_close(&mut reader, "HTTP/1.1 200 OK");
}

#[test]
fn http10_closes_by_default() {
    let mut reader = client(test_config());
    reader.send(b"GET /health HTTP/1.0\r\n\r\n");
    assert_close(&mut reader, "HTTP/1.1 200 OK");
}

#[test]
fn http10_explicit_keep_alive_is_announced_and_reused() {
    let mut reader = client(test_config());
    for _ in 0..2 {
        reader.send(b"GET /health HTTP/1.0\r\nConnection: keep-alive\r\n\r\n");
        let response = reader.read_response(false);
        assert_eq!(response.header("Connection"), "keep-alive");
        assert_eq!(response.body, b"ok\n");
    }
    reader.send(b"GET /health HTTP/1.0\r\n\r\n");
    assert_close(&mut reader, "HTTP/1.1 200 OK");
}

#[test]
fn close_wins_across_tokens_case_and_repeated_headers() {
    for value in [
        "keep-alive, close",
        "CLOSE",
        "keep-alive\r\nConnection: close",
    ] {
        let mut reader = client(test_config());
        reader.send(
            format!("GET /health HTTP/1.1\r\nHost: x\r\nConnection: {value}\r\n\r\n").as_bytes(),
        );
        assert_close(&mut reader, "HTTP/1.1 200 OK");
    }
}

#[test]
fn max_requests_closes_on_the_third_response() {
    let mut reader = client(Config {
        max_requests: 3,
        ..test_config()
    });
    reader.send(&GET.repeat(4));
    for _ in 0..2 {
        assert_eq!(
            reader.read_response(false).optional_header("Connection"),
            None
        );
    }
    assert_close(&mut reader, "HTTP/1.1 200 OK");
}

#[test]
fn initial_idle_timeout_closes_without_a_response() {
    let mut reader = client(test_config());
    let started = Instant::now();
    reader.assert_eof();
    timeout_margin(started.elapsed());
}

#[test]
fn idle_after_response_closes_without_an_extra_response() {
    let mut reader = client(test_config());
    reader.send(GET);
    assert_eq!(reader.read_response(false).body, b"ok\n");
    let started = Instant::now();
    reader.assert_eof();
    timeout_margin(started.elapsed());
}

#[test]
fn incomplete_request_times_out_with_408() {
    let mut reader = client(test_config());
    let started = Instant::now();
    reader.send(b"GET /health HTTP/1.1\r\nHost:");
    assert_close(&mut reader, "HTTP/1.1 408 Request Timeout");
    timeout_margin(started.elapsed());
}

#[test]
fn slowloris_activity_does_not_renew_the_deadline() {
    let mut reader = client(Config {
        request_timeout: Duration::from_millis(500),
        ..test_config()
    });
    reader.send(b"G");
    let mut writer = reader.stream.try_clone().unwrap();
    let running = Arc::new(AtomicBool::new(true));
    let active = running.clone();
    let sender = thread::spawn(move || {
        while active.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            if writer.write_all(b"x").is_err() {
                break;
            }
        }
    });
    let started = Instant::now();
    assert_close(&mut reader, "HTTP/1.1 408 Request Timeout");
    let elapsed = started.elapsed();
    running.store(false, Ordering::Relaxed);
    sender.join().unwrap();
    assert!(
        elapsed >= Duration::from_millis(350) && elapsed < Duration::from_secs(2),
        "{elapsed:?}"
    );
}

#[test]
fn parse_error_after_valid_request_terminates_the_pipeline() {
    let mut reader = client(test_config());
    reader.send(GET);
    assert_eq!(reader.read_response(false).body, b"ok\n");
    let mut tail = b"GET /\r\n\r\n".to_vec();
    tail.extend_from_slice(GET);
    reader.send(&tail);
    assert_close(&mut reader, "HTTP/1.1 400 Bad Request");
    // A write may succeed into the local buffer after FIN; no further response is allowed.
    let _ = reader.stream.write_all(GET);
}

#[test]
fn half_close_still_returns_all_complete_buffered_requests() {
    let mut reader = client(test_config());
    reader.send(&GET.repeat(2));
    reader.stream.shutdown(Shutdown::Write).unwrap();
    for _ in 0..2 {
        assert_eq!(reader.read_response(false).body, b"ok\n");
    }
    reader.assert_eof();
}

#[test]
fn head_then_get_are_aligned_on_one_connection() {
    let mut reader = client(test_config());
    let mut bytes = b"HEAD /health HTTP/1.1\r\nHost: x\r\n\r\n".to_vec();
    bytes.extend_from_slice(GET);
    reader.send(&bytes);
    let head = reader.read_response(true);
    let get = reader.read_response(false);
    assert_eq!(head.headers, get.headers);
    assert_eq!(head.header("Content-Length"), "3");
    assert!(head.body.is_empty());
    assert_eq!(get.body, b"ok\n");
}

#[test]
fn header_bomb_431_survives_twenty_connections() {
    let addr = start_server_with_config(test_config());
    let mut bomb = b"GET / HTTP/1.1\r\nX: ".to_vec();
    bomb.resize(20_000, b'a');
    for _ in 0..20 {
        let mut reader = ResponseReader::new(connect(addr));
        reader.send(&bomb);
        assert_close(&mut reader, "HTTP/1.1 431 Request Header Fields Too Large");
    }
}

#[test]
fn framing_errors_and_timeouts_always_close() {
    let oversized = format!(
        "POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY + 1
    );
    let bomb = vec![b'x'; 20_000];
    for (request, status) in [
        (b"GET /\r\n\r\n".as_slice(), "HTTP/1.1 400 Bad Request"),
        (
            b"GET /health HTTP/1.1\r\nHost:".as_slice(),
            "HTTP/1.1 408 Request Timeout",
        ),
        (oversized.as_bytes(), "HTTP/1.1 413 Payload Too Large"),
        (
            bomb.as_slice(),
            "HTTP/1.1 431 Request Header Fields Too Large",
        ),
    ] {
        let mut reader = client(test_config());
        reader.send(request);
        assert_close(&mut reader, status);
    }
}

#[test]
fn request_deadline_starts_after_idle_and_resets_for_the_next_request() {
    let mut reader = client(Config {
        idle_timeout: Duration::from_millis(800),
        request_timeout: Duration::from_millis(400),
        ..test_config()
    });
    for _ in 0..2 {
        thread::sleep(Duration::from_millis(250));
        reader.send(&GET[..15]);
        thread::sleep(Duration::from_millis(200));
        reader.send(&GET[15..]);
        assert_eq!(reader.read_response(false).body, b"ok\n");
    }
}

#[test]
fn body_stall_and_head_stall_use_request_timeout() {
    let mut reader = client(test_config());
    reader.send(b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\na");
    assert_close(&mut reader, "HTTP/1.1 408 Request Timeout");
    let mut reader = client(test_config());
    reader.send(b"HEAD /health HTTP/1.1\r\nHost:");
    let response = reader.read_response(true);
    assert_eq!(response.status, "HTTP/1.1 408 Request Timeout");
    assert_eq!(response.header("Connection"), "close");
    assert_eq!(response.header("Content-Length"), "15");
    reader.assert_eof();
}

#[test]
fn post_body_followed_by_get_and_router_errors_preserve_framing() {
    let mut reader = client(test_config());
    let mut requests = b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\n\xff\0aGET /nope HTTP/1.1\r\nHost: x\r\n\r\nDELETE /health HTTP/1.1\r\nHost: x\r\n\r\n".to_vec();
    requests.extend_from_slice(GET);
    reader.send(&requests);
    assert_eq!(reader.read_response(false).body, b"\xff\0a");
    assert_eq!(reader.read_response(false).status, "HTTP/1.1 404 Not Found");
    assert_eq!(
        reader.read_response(false).status,
        "HTTP/1.1 405 Method Not Allowed"
    );
    assert_eq!(reader.read_response(false).body, b"ok\n");
}
