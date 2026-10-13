mod support;
use proptest::prelude::*;
use rust_serv::{
    connection::Config,
    http::{
        headers::Headers,
        range::{ByteRange, RangeError},
        request::{Method, Request, Version},
        response::Status,
    },
    routes,
    server::{Execution, Server},
    static_files::StaticFiles,
};
use std::{
    fs,
    net::SocketAddr,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};
use support::{ResponseReader, connect};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "rust-range-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        fs::write(p.join("data.bin"), (0..=255).collect::<Vec<u8>>()).unwrap();
        fs::write(p.join("empty"), []).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn parser_table_at_least_fifteen_cases() {
    use ByteRange::*;
    for (text, expected) in [
        ("bytes=0-99", Ok(FromTo(0, 99))),
        ("bytes=100-", Ok(From(100))),
        ("bytes=-50", Ok(Suffix(50))),
        ("bytes=0-0", Ok(FromTo(0, 0))),
        ("bytes=-0", Ok(Suffix(0))),
        ("BYTES=0001-0002", Ok(FromTo(1, 2))),
        (" bytes=1-2\t", Ok(FromTo(1, 2))),
        ("bytes=18446744073709551615-", Ok(From(u64::MAX))),
        ("bytes=0-18446744073709551615", Ok(FromTo(0, u64::MAX))),
        ("bytes=18446744073709551616-", Err(RangeError::Invalid)),
        ("bytes=-18446744073709551616", Err(RangeError::Invalid)),
        ("bytes=", Err(RangeError::Invalid)),
        ("bytes=-", Err(RangeError::Invalid)),
        ("bytes=2-1", Err(RangeError::Invalid)),
        ("bytes=+1-2", Err(RangeError::Invalid)),
        ("bytes=1--2", Err(RangeError::Invalid)),
        ("bytes=a-b", Err(RangeError::Invalid)),
        ("bytes=1-2-3", Err(RangeError::Invalid)),
        ("bytes= 1-2", Err(RangeError::Invalid)),
        ("bytes=1 -2", Err(RangeError::Invalid)),
        ("bytes=1- 2", Err(RangeError::Invalid)),
        ("bytes=1.0-2", Err(RangeError::Invalid)),
        ("bytes", Err(RangeError::Invalid)),
        ("items=0-1", Err(RangeError::Unsupported)),
        ("bytes=0-1,5-6", Err(RangeError::Unsupported)),
    ] {
        assert_eq!(text.parse::<ByteRange>(), expected, "{text}");
    }
}
#[test]
fn resolution_table_inclusive_end_and_extremes() {
    use ByteRange::*;
    for (range, total, expected) in [
        (FromTo(0, 99), 256, Some((0, 99, 100))),
        (From(100), 256, Some((100, 255, 156))),
        (Suffix(50), 256, Some((206, 255, 50))),
        (Suffix(500), 256, Some((0, 255, 256))),
        (FromTo(255, 999), 256, Some((255, 255, 1))),
        (FromTo(0, u64::MAX), 256, Some((0, 255, 256))),
        (From(256), 256, None),
        (From(u64::MAX), 256, None),
        (Suffix(0), 256, None),
        (Suffix(1), 0, None),
        (From(0), 0, None),
        (FromTo(2, 1), 256, None),
        (From(0), 1, Some((0, 0, 1))),
        (
            Suffix(u64::MAX),
            u64::MAX,
            Some((0, u64::MAX - 1, u64::MAX)),
        ),
        (
            From(u64::MAX - 1),
            u64::MAX,
            Some((u64::MAX - 1, u64::MAX - 1, 1)),
        ),
    ] {
        assert_eq!(
            range.resolve(total).map(|r| (r.start, r.end, r.len)),
            expected,
            "{range:?}, {total}"
        );
    }
}
proptest! {
 #[test]
 fn resolved_ranges_never_escape_file(total in any::<u64>(),start in any::<u64>(),end in any::<u64>(),suffix in any::<u64>()) {
  for range in [ByteRange::From(start),ByteRange::FromTo(start,end),ByteRange::Suffix(suffix)] {
   if let Some(r)=range.resolve(total) {prop_assert!(r.start<=r.end);prop_assert!(r.end<total);prop_assert!(r.len>0);prop_assert!(r.len<=total);prop_assert_eq!(r.start.checked_add(r.len),r.end.checked_add(1));}
  }
 }
 #[test]
 fn single_range_matches_vec_slice(total in 1usize..4096,start in 0usize..8192,end in 0usize..8192) {
  let source:Vec<_>=(0..total).map(|i|(i%251) as u8).collect();
  let result=ByteRange::FromTo(start as u64,end as u64).resolve(total as u64);
  if let Some(r)=result {prop_assert_eq!(r.len as usize, source[r.start as usize..=r.end as usize].len());} else {prop_assert!(start>=total || end<start);}
 }
}
fn req(method: Method, range: Option<&str>, if_range: Option<&str>) -> Request {
    let mut headers = Headers::new();
    if let Some(v) = range {
        headers.push("Range".into(), v.into());
    }
    if let Some(v) = if_range {
        headers.push("If-Range".into(), v.into());
    }
    Request {
        method,
        target: "/data.bin".into(),
        version: Version::Http11,
        headers,
        body: vec![],
    }
}
#[test]
fn if_range_validator_changes_with_metadata_and_weak_dates_fall_back() {
    let f = Fixture::new();
    let files = StaticFiles::new(&f.0).unwrap();
    let whole = files.serve(&req(Method::Get, None, None));
    let etag = whole.headers.get("ETag").unwrap();
    assert_eq!(
        files
            .serve(&req(Method::Get, Some("bytes=1-2"), Some(etag)))
            .status,
        Status::PartialContent
    );
    for value in [
        "\"stale\"",
        &format!("W/{etag}"),
        whole.headers.get("Last-Modified").unwrap(),
        "garbage",
    ] {
        let r = files.serve(&req(Method::Get, Some("bytes=1-2"), Some(value)));
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.body.len(), Some(256));
    }
    fs::write(f.0.join("data.bin"), b"new version").unwrap();
    let updated = files.serve(&req(Method::Get, None, None));
    assert_ne!(updated.headers.get("ETag"), Some(etag));
    assert_eq!(
        files
            .serve(&req(Method::Get, Some("bytes=1-2"), Some(etag)))
            .status,
        Status::Ok
    );
}
#[test]
fn head_ignores_range_and_304_precedes_range() {
    let f = Fixture::new();
    let files = StaticFiles::new(&f.0).unwrap();
    let normal = files.serve(&req(Method::Get, None, None));
    let head = files.serve(&req(Method::Head, Some("bytes=999-"), None));
    assert_eq!(head.status, Status::Ok);
    assert_eq!(head.body.len(), Some(256));
    assert!(head.headers.get("Content-Range").is_none());
    let mut conditional = req(Method::Get, Some("bytes=999-"), None);
    conditional.headers.push(
        "If-Modified-Since".into(),
        normal.headers.get("Last-Modified").unwrap().into(),
    );
    let r = files.serve(&conditional);
    assert_eq!(r.status, Status::NotModified);
    assert!(r.headers.get("Content-Range").is_none());
}
#[test]
fn multiple_ranges_and_duplicates_are_ignored_without_multipart() {
    let f = Fixture::new();
    let files = StaticFiles::new(&f.0).unwrap();
    for range in ["bytes=0-1,4-5", "items=0-1"] {
        assert_eq!(
            files.serve(&req(Method::Get, Some(range), None)).status,
            Status::Ok
        );
    }
    let mut r = req(Method::Get, Some("bytes=0-1"), None);
    r.headers.push("Range".into(), "bytes=5-6".into());
    assert_eq!(files.serve(&r).status, Status::Ok);
    let etag = files
        .serve(&req(Method::Get, None, None))
        .headers
        .get("ETag")
        .unwrap()
        .to_owned();
    let mut r = req(Method::Get, Some("bytes=0-1"), Some(&etag));
    r.headers.push("If-Range".into(), etag);
    assert_eq!(files.serve(&r).status, Status::Ok);
}
#[test]
fn mtime_nanoseconds_change_etag_at_equal_length() {
    use std::fs::{File, FileTimes};
    use std::time::{SystemTime, UNIX_EPOCH};
    let f = Fixture::new();
    let file = File::options()
        .write(true)
        .open(f.0.join("data.bin"))
        .unwrap();
    let files = StaticFiles::new(&f.0).unwrap();
    let date: SystemTime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    file.set_times(FileTimes::new().set_modified(date + Duration::from_nanos(1)))
        .unwrap();
    let first = files.serve(&req(Method::Get, None, None));
    file.set_times(FileTimes::new().set_modified(date + Duration::from_nanos(2)))
        .unwrap();
    let second = files.serve(&req(Method::Get, None, None));
    assert_eq!(
        first.headers.get("Last-Modified"),
        second.headers.get("Last-Modified")
    );
    assert_ne!(first.headers.get("ETag"), second.headers.get("ETag"));
}
fn modes() -> Vec<Execution> {
    let mut m = vec![
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
    m.extend([
        Execution::Epoll {
            edge_triggered: false,
        },
        Execution::Epoll {
            edge_triggered: true,
        },
    ]);
    m
}
struct Running {
    addr: SocketAddr,
    stop: rust_serv::server::ShutdownHandle,
    join: Option<thread::JoinHandle<std::io::Result<()>>>,
}
impl Running {
    fn new(f: &Fixture, mode: Execution) -> Self {
        let s = Server::bind_with_config(
            "127.0.0.1:0",
            routes::static_router(StaticFiles::new(&f.0).unwrap()),
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
        Self {
            addr: s.local_addr().unwrap(),
            stop: s.shutdown_handle(),
            join: Some(thread::spawn(move || s.run())),
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        self.stop.request_shutdown();
        self.join.take().unwrap().join().unwrap().unwrap();
    }
}
#[test]
fn ranges_all_transports_pipeline_matches_bytes_and_416_recovers() {
    for mode in modes() {
        let f = Fixture::new();
        let server = Running::new(&f, mode);
        let mut reader = ResponseReader::new(connect(server.addr));
        for (range, start, len) in [
            ("bytes=0-99", 0, 100),
            ("bytes=100-", 100, 156),
            ("bytes=-50", 206, 50),
            ("bytes=-999", 0, 256),
            ("bytes=255-999", 255, 1),
        ] {
            reader.send(
                format!("GET /data.bin HTTP/1.1\r\nHost: x\r\nRange: {range}\r\n\r\n").as_bytes(),
            );
            let r = reader.read_response(false);
            assert_eq!(r.status, "HTTP/1.1 206 Partial Content", "{mode:?}");
            assert_eq!(r.header("Content-Length"), len.to_string());
            assert_eq!(
                r.header("Content-Range"),
                format!("bytes {}-{}/256", start, start + len - 1)
            );
            assert_eq!(r.header("Accept-Ranges"), "bytes");
            assert_eq!(
                r.body,
                (start..start + len).map(|i| i as u8).collect::<Vec<_>>()
            );
        }
        for path in ["data.bin", "empty"] {
            reader.send(
                format!("GET /{path} HTTP/1.1\r\nHost: x\r\nRange: bytes=999-\r\n\r\n").as_bytes(),
            );
            let r = reader.read_response(false);
            assert_eq!(r.status, "HTTP/1.1 416 Range Not Satisfiable");
            assert_eq!(
                r.header("Content-Range"),
                format!("bytes */{}", if path == "empty" { 0 } else { 256 })
            );
            assert_eq!(r.header("Content-Length"), "0");
        }
        reader.send(b"GET /data.bin HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(
            reader.read_response(false).body,
            (0..=255).collect::<Vec<u8>>()
        );
        reader.assert_eof();
    }
}
#[test]
fn range_repeated_connections_large_offsets_and_if_range_on_wire() {
    for mode in modes() {
        let f = Fixture::new();
        let mut bytes = vec![0; 512 * 1024];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        fs::write(f.0.join("data.bin"), &bytes).unwrap();
        let server = Running::new(&f, mode);
        let mut r = ResponseReader::new(connect(server.addr));
        r.send(
            b"HEAD /data.bin HTTP/1.1\r\nHost: x\r\nRange: bytes=1-2\r\nConnection: close\r\n\r\n",
        );
        let h = r.read_response(true);
        assert_eq!(h.status, "HTTP/1.1 200 OK");
        let etag = h.header("ETag").to_owned();
        r.assert_eof();
        for valid in [true, false] {
            let mut r = ResponseReader::new(connect(server.addr));
            r.send(format!("GET /data.bin HTTP/1.1\r\nHost: x\r\nRange: bytes=400000-499999\r\nIf-Range: {}\r\nConnection: close\r\n\r\n",if valid{&etag}else{"\"stale\""}).as_bytes());
            let body = r.read_response(false);
            assert_eq!(
                body.body,
                if valid {
                    &bytes[400000..500000]
                } else {
                    &bytes[..]
                }
            );
            r.assert_eof();
        }
    }
}
#[test]
fn malformed_and_unsatisfiable_ranges_are_416_on_wire() {
    let f = Fixture::new();
    let server = Running::new(
        &f,
        Execution::Tokio {
            max_connections: 16,
        },
    );
    let mut r = ResponseReader::new(connect(server.addr));
    for range in [
        "bytes=2-1",
        "bytes=garbage",
        "bytes=-0",
        "bytes=256-",
        "bytes=18446744073709551616-",
    ] {
        r.send(format!("GET /data.bin HTTP/1.1\r\nHost: x\r\nRange: {range}\r\n\r\n").as_bytes());
        let res = r.read_response(false);
        assert_eq!(res.status, "HTTP/1.1 416 Range Not Satisfiable");
        assert_eq!(res.header("Content-Range"), "bytes */256");
    }
}
