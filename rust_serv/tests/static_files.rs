mod support;
use rust_serv::{
    http::{
        headers::Headers,
        request::{Method, Request, Version},
        response::{Response, Status},
    },
    routes,
    server::Server,
    static_files::{StaticFiles, mime_for},
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    base: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join(format!(
            "rust-serv-static-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let f = Self { base };
        fs::create_dir(f.root()).unwrap();
        fs::create_dir(f.base.join("public-secret")).unwrap();
        fs::write(f.base.join("secret.txt"), "TOP SECRET").unwrap();
        fs::write(f.base.join("public-secret/leak.txt"), "TOP SECRET").unwrap();
        for (name, bytes) in [
            ("index.html", "<h1>index</h1>"),
            ("style.css", "body{}"),
            ("app.js", "console.log(1)"),
            ("logo.png", "png"),
            ("icon.svg", "<svg/>"),
            ("app.wasm", "wasm"),
            ("café.txt", "café"),
            ("a b.txt", "space"),
            ("100%.txt", "percent"),
            ("empty.txt", ""),
            ("health", "shadow"),
            (".secret", "TOP SECRET"),
        ] {
            fs::write(f.root().join(name), bytes).unwrap();
        }
        fs::write(f.root().join("data.bin"), (0..=255).collect::<Vec<u8>>()).unwrap();
        fs::create_dir(f.root().join("docs")).unwrap();
        fs::write(f.root().join("docs/index.html"), "docs").unwrap();
        fs::create_dir(f.root().join("emptydir")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("index.html", f.root().join("link-in")).unwrap();
            symlink("../secret.txt", f.root().join("link-out")).unwrap();
            symlink("../public-secret", f.root().join("link-dir")).unwrap();
            symlink("missing", f.root().join("broken")).unwrap();
        }
        f
    }
    fn root(&self) -> PathBuf {
        self.base.join("public")
    }
    fn files(&self) -> StaticFiles {
        StaticFiles::new(self.root()).unwrap()
    }
    fn serve(&self, target: &str) -> Response {
        self.files().serve(&request(Method::Get, target))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}
fn request(method: Method, target: &str) -> Request {
    Request {
        method,
        target: target.into(),
        version: Version::Http11,
        headers: Headers::new(),
        body: vec![],
    }
}
#[test]
fn traversal_table() {
    let f = Fixture::new();
    assert_eq!(fs::read(f.base.join("secret.txt")).unwrap(), b"TOP SECRET");
    for (path, status) in [
        ("/../secret.txt", Status::Forbidden),
        ("/a/../../secret.txt", Status::Forbidden),
        ("/%2e%2e/secret.txt", Status::Forbidden),
        ("/.%2e/secret.txt", Status::Forbidden),
        ("/..%2fsecret.txt", Status::BadRequest),
        ("/%2e%2e%2fsecret.txt", Status::BadRequest),
        ("/%00", Status::BadRequest),
        ("/x%00.html", Status::BadRequest),
        ("/./index.html", Status::Forbidden),
        ("//index.html", Status::NotFound),
        ("/link-out", Status::Forbidden),
        ("/link-dir/leak.txt", Status::Forbidden),
        ("/.secret", Status::NotFound),
        ("/%5csecret", Status::BadRequest),
        ("/docs//", Status::NotFound),
        ("/%zz", Status::BadRequest),
        ("/broken", Status::NotFound),
    ] {
        let r = f.serve(path);
        assert_eq!(r.status, status, "{path}");
        assert!(!r.body.windows(10).any(|w| w == b"TOP SECRET"));
        assert_eq!(r.headers.get("Location"), None);
    }
}
#[test]
fn mime_table() {
    for (name, mime) in [
        ("a.HTML", "text/html; charset=utf-8"),
        ("a.css", "text/css; charset=utf-8"),
        ("a.js", "text/javascript; charset=utf-8"),
        ("a.json", "application/json"),
        ("a.txt", "text/plain; charset=utf-8"),
        ("a.png", "image/png"),
        ("a.jpg", "image/jpeg"),
        ("a.jpeg", "image/jpeg"),
        ("a.gif", "image/gif"),
        ("a.webp", "image/webp"),
        ("a.svg", "image/svg+xml"),
        ("a.ico", "image/x-icon"),
        ("a.woff2", "font/woff2"),
        ("a.pdf", "application/pdf"),
        ("a.wasm", "application/wasm"),
        ("a.mp4", "video/mp4"),
        ("a.webm", "video/webm"),
        ("a", "application/octet-stream"),
        (".html", "application/octet-stream"),
        ("a.tar.gz", "application/octet-stream"),
    ] {
        assert_eq!(mime_for(std::path::Path::new(name)), mime);
    }
}
#[test]
fn root_validation() {
    let f = Fixture::new();
    assert!(StaticFiles::new(f.base.join("missing")).is_err());
    assert!(StaticFiles::new(f.root().join("index.html")).is_err());
}
#[test]
fn directories_encoding_errors_and_methods() {
    let f = Fixture::new();
    for (path, body) in [
        ("/", "<h1>index</h1>"),
        ("/docs/", "docs"),
        ("/caf%C3%A9.txt", "café"),
        ("/a%20b.txt", "space"),
        ("/100%25.txt", "percent"),
        ("/link-in", "<h1>index</h1>"),
    ] {
        let r = f.serve(path);
        assert_eq!(r.status, Status::Ok);
        assert_eq!(r.body, body.as_bytes());
    }
    for (path, status) in [
        ("/missing", Status::NotFound),
        ("/index.html/extra", Status::NotFound),
        ("/emptydir/", Status::Forbidden),
    ] {
        assert_eq!(f.serve(path).status, status);
    }
    for (path, location) in [("/docs", "/docs/"), ("/docs?x=1", "/docs/?x=1")] {
        let r = f.serve(path);
        assert_eq!(r.status, Status::MovedPermanently);
        assert_eq!(r.headers.get("Location"), Some(location));
    }
    let r = f.files().serve(&request(Method::Post, "/index.html"));
    assert_eq!(r.status, Status::MethodNotAllowed);
    assert_eq!(r.headers.get("Allow"), Some("GET, HEAD"));
}
#[test]
fn conditional_and_head() {
    let f = Fixture::new();
    let files = f.files();
    let get = files.serve(&request(Method::Get, "/index.html"));
    let head = files.serve(&request(Method::Head, "/index.html"));
    assert_eq!(
        get.headers.iter().collect::<Vec<_>>(),
        head.headers.iter().collect::<Vec<_>>()
    );
    let mut wire = vec![];
    head.write_head_to(&mut wire).unwrap();
    assert!(wire.ends_with(b"\r\n\r\n"));
    for (date, status) in [
        (
            get.headers.get("Last-Modified").unwrap(),
            Status::NotModified,
        ),
        ("Thu, 01 Jan 1970 00:00:00 GMT", Status::Ok),
        ("Fri, 01 Jan 2100 00:00:00 GMT", Status::NotModified),
        ("trash", Status::Ok),
    ] {
        for method in [Method::Get, Method::Head] {
            let mut req = request(method, "/index.html");
            req.headers.push("If-Modified-Since".into(), date.into());
            let r = files.serve(&req);
            assert_eq!(r.status, status);
            if status == Status::NotModified {
                let mut wire = vec![];
                r.write_to(&mut wire).unwrap();
                assert!(!String::from_utf8(wire).unwrap().contains("Content-Length"));
                assert!(r.body.is_empty());
            }
        }
    }
}
#[test]
fn binaries_limits_and_empty() {
    let f = Fixture::new();
    assert_eq!(f.serve("/data.bin").body, (0..=255).collect::<Vec<u8>>());
    let large = (0..20 * 1024 * 1024)
        .map(|i| (i % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(f.root().join("large.bin"), &large).unwrap();
    assert_eq!(f.serve("/large.bin").body, large);
    let file = fs::File::create(f.root().join("oversize")).unwrap();
    file.set_len(128 * 1024 * 1024 + 1).unwrap();
    assert_eq!(f.serve("/oversize").status, Status::InternalServerError);
    let mut wire = vec![];
    f.serve("/empty.txt").write_to(&mut wire).unwrap();
    assert!(
        String::from_utf8(wire)
            .unwrap()
            .contains("Content-Length: 0")
    );
}
#[test]
fn fallback_precedence() {
    let f = Fixture::new();
    let router = routes::static_router(f.files());
    assert_eq!(
        router.handle(&request(Method::Get, "/health")).body,
        b"ok\n"
    );
    assert_eq!(
        router.handle(&request(Method::Post, "/health")).status,
        Status::MethodNotAllowed
    );
    assert_eq!(
        router.handle(&request(Method::Get, "/nope")).status,
        Status::NotFound
    );
}
#[test]
fn no_body_statuses() {
    for status in [Status::NoContent, Status::NotModified] {
        let mut wire = vec![];
        Response::new(status)
            .header("Content-Length", "999")
            .body("forbidden body")
            .write_to(&mut wire)
            .unwrap();
        assert!(wire.ends_with(b"\r\n\r\n"));
        assert!(!String::from_utf8(wire).unwrap().contains("Content-Length"));
    }
}
#[test]
fn socket_keep_alive_after_304() {
    let f = Fixture::new();
    let root = f.root();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let server = Server::bind(
            "127.0.0.1:0",
            routes::static_router(StaticFiles::new(root).unwrap()),
        )
        .unwrap();
        tx.send(server.local_addr().unwrap()).unwrap();
        server.run().unwrap();
    });
    let mut reader = support::ResponseReader::new(support::connect(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
    ));
    reader.send(b"GET /index.html HTTP/1.1\r\nHost: x\r\n\r\n");
    let first = reader.read_response(false);
    assert_eq!(first.body, b"<h1>index</h1>");
    assert_eq!(first.header("X-Content-Type-Options"), "nosniff");
    reader.send(
        format!(
            "GET /index.html HTTP/1.1\r\nHost: x\r\nIf-Modified-Since: {}\r\n\r\n",
            first.header("Last-Modified")
        )
        .as_bytes(),
    );
    let cached = reader.read_response(false);
    assert_eq!(cached.status, "HTTP/1.1 304 Not Modified");
    assert!(cached.optional_header("Content-Length").is_none());
    for path in ["/style.css", "/app.js", "/data.bin"] {
        reader.send(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes());
        assert_eq!(reader.read_response(false).status, "HTTP/1.1 200 OK");
    }
    reader.send(b"HEAD /index.html HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let head = reader.read_response(true);
    assert_eq!(
        head.header("Content-Length"),
        first.header("Content-Length")
    );
    reader.assert_eof();
}
#[cfg(unix)]
#[test]
fn permissions() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let p = f.root().join("denied");
    fs::write(&p, "secret").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o0)).unwrap();
    if fs::File::open(&p).is_ok() {
        eprintln!("permission test skipped: privileged user");
        return;
    }
    assert_eq!(f.serve("/denied").status, Status::Forbidden);
}

macro_rules! wire_case {
    ($name:ident, $method:literal, $path:literal, $code:literal) => {
        #[test]
        fn $name() {
            let fixture = Fixture::new();
            let root = fixture.root();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let server = Server::bind(
                    "127.0.0.1:0",
                    routes::static_router(StaticFiles::new(root).unwrap()),
                )
                .unwrap();
                tx.send(server.local_addr().unwrap()).unwrap();
                server.run().unwrap();
            });
            let mut reader = support::ResponseReader::new(support::connect(
                rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            ));
            reader.send(
                concat!(
                    $method,
                    " ",
                    $path,
                    " HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
                )
                .as_bytes(),
            );
            let response = reader.read_response($method == "HEAD");
            assert_eq!(response.status.split_whitespace().nth(1), Some($code));
            assert_eq!(response.header("X-Content-Type-Options"), "nosniff");
            if $code == "200" && $method == "GET" {
                let decoded = rust_serv::http::uri::decode_path($path).unwrap();
                let relative = if decoded == "/" {
                    "index.html"
                } else if decoded == "/docs/" {
                    "docs/index.html"
                } else {
                    &decoded[1..]
                };
                assert_eq!(
                    response.body,
                    fs::read(fixture.root().join(relative)).unwrap()
                );
                assert_eq!(
                    response.header("Content-Type"),
                    mime_for(&fs::canonicalize(fixture.root().join(relative)).unwrap())
                );
            }
            reader.assert_eof();
        }
    };
}
wire_case!(wire_index, "GET", "/index.html", "200");
wire_case!(wire_root, "GET", "/", "200");
wire_case!(wire_css, "GET", "/style.css", "200");
wire_case!(wire_js, "GET", "/app.js", "200");
wire_case!(wire_png, "GET", "/logo.png", "200");
wire_case!(wire_svg, "GET", "/icon.svg", "200");
wire_case!(wire_wasm, "GET", "/app.wasm", "200");
wire_case!(wire_head, "HEAD", "/index.html", "200");
wire_case!(wire_missing, "GET", "/missing", "404");
wire_case!(wire_not_directory, "GET", "/index.html/extra", "404");
wire_case!(wire_post, "POST", "/index.html", "405");
wire_case!(wire_link_in, "GET", "/link-in", "200");
wire_case!(wire_redirect, "GET", "/docs", "301");
wire_case!(wire_redirect_query, "GET", "/docs?x=1", "301");
wire_case!(wire_docs, "GET", "/docs/", "200");
wire_case!(wire_emptydir, "GET", "/emptydir/", "403");
wire_case!(wire_unicode, "GET", "/caf%C3%A9.txt", "200");
wire_case!(wire_space, "GET", "/a%20b.txt", "200");
wire_case!(wire_percent, "GET", "/100%25.txt", "200");
wire_case!(wire_binary, "GET", "/data.bin", "200");
wire_case!(wire_empty, "GET", "/empty.txt", "200");
