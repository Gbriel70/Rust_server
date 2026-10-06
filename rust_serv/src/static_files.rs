//! Static files beneath a canonical root. Local filesystem writers must be trusted:
//! canonicalize followed by open still has a TOCTOU window (openat2 can close it).
use crate::http::{
    date::{format_http_date, parse_http_date},
    request::{Method, Request},
    response::{Response, Status},
    uri::decode_path,
};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
const MAX_SIZE: u64 = 128 * 1024 * 1024;
pub struct StaticFiles {
    root: PathBuf,
}
impl StaticFiles {
    pub fn new(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "static root is not a directory",
            ));
        }
        Ok(Self { root })
    }
    fn resolve(&self, raw: &str) -> Result<PathBuf, Status> {
        let decoded = decode_path(raw).map_err(|_| Status::BadRequest)?;
        if !decoded.starts_with('/') {
            return Err(Status::BadRequest);
        }
        let segments: Vec<_> = decoded[1..].split('/').collect();
        if segments.iter().any(|s| matches!(*s, "." | "..")) {
            return Err(Status::Forbidden);
        }
        if segments
            .iter()
            .enumerate()
            .any(|(i, s)| s.is_empty() && i + 1 < segments.len())
        {
            return Err(Status::NotFound);
        }
        if segments.iter().any(|s| s.starts_with('.')) {
            return Err(Status::NotFound);
        }
        let mut candidate = self.root.clone();
        for s in segments {
            candidate.push(s);
        }
        self.beneath(&candidate)
    }
    fn beneath(&self, path: &Path) -> Result<PathBuf, Status> {
        let path = fs::canonicalize(path).map_err(io_status)?;
        if !path.starts_with(&self.root) {
            return Err(Status::Forbidden);
        }
        Ok(path)
    }
    pub fn serve(&self, request: &Request) -> Response {
        self.serve_inner(request)
            .unwrap_or_else(|status| Response::text(status, status.reason()))
            .header("X-Content-Type-Options", "nosniff")
    }
    fn serve_inner(&self, request: &Request) -> Result<Response, Status> {
        if !matches!(request.method, Method::Get | Method::Head) {
            return Ok(Response::new(Status::MethodNotAllowed).header("Allow", "GET, HEAD"));
        }
        let mut path = self.resolve(request.path())?;
        if fs::metadata(&path).map_err(io_status)?.is_dir() {
            if !request.path().ends_with('/') {
                let mut location = format!("{}/", request.path());
                if let Some(query) = request.query() {
                    location.push('?');
                    location.push_str(query);
                }
                return Ok(Response::new(Status::MovedPermanently).header("Location", location));
            }
            path = self.beneath(&path.join("index.html")).map_err(|s| {
                if s == Status::NotFound {
                    Status::Forbidden
                } else {
                    s
                }
            })?;
        }
        let file = File::open(&path).map_err(io_status)?;
        let metadata = file.metadata().map_err(io_status)?;
        if !metadata.is_file() {
            return Err(Status::Forbidden);
        }
        if metadata.len() > MAX_SIZE {
            return Err(Status::InternalServerError);
        }
        let modified = metadata.modified().map_err(io_status)?;
        let last_modified = format_http_date(modified);
        // The formatted validator represents the mtime truncated to whole seconds.
        let unmodified = request
            .headers
            .get("If-Modified-Since")
            .and_then(parse_http_date)
            .zip(parse_http_date(&last_modified))
            .is_some_and(|(since, mtime)| since >= mtime);
        let response = Response::new(if unmodified {
            Status::NotModified
        } else {
            Status::Ok
        })
        .header("Content-Type", mime_for(&path))
        .header("Last-Modified", last_modified)
        .header("Cache-Control", "no-cache");
        if unmodified {
            return Ok(response);
        }
        // HEAD intentionally reads too; Phase 10 introduces a declared body length.
        let mut body = Vec::new();
        file.take(MAX_SIZE + 1)
            .read_to_end(&mut body)
            .map_err(io_status)?;
        if body.len() as u64 > MAX_SIZE {
            return Err(Status::InternalServerError);
        }
        Ok(response.body(body))
    }
}
fn io_status(error: io::Error) -> Status {
    match error.kind() {
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Status::NotFound,
        io::ErrorKind::PermissionDenied => Status::Forbidden,
        _ => Status::InternalServerError,
    }
}
pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "txt" => "text/plain; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn lexical_and_canonical_resolution() {
        let base = std::env::temp_dir().join(format!(
            "static-resolution-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&base).unwrap();
        let temp = Temp(base);
        let root = temp.0.join("public");
        fs::create_dir(&root).unwrap();
        fs::create_dir(temp.0.join("public-secret")).unwrap();
        fs::write(temp.0.join("secret.txt"), "TOP SECRET").unwrap();
        fs::write(temp.0.join("public-secret/leak.txt"), "TOP SECRET").unwrap();
        fs::write(root.join("index.html"), "index").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("../secret.txt", root.join("link-out")).unwrap();
            symlink("../public-secret", root.join("link-dir")).unwrap();
        }
        let files = StaticFiles::new(root).unwrap();
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
            ("/.secret", Status::NotFound),
            ("/%5csecret", Status::BadRequest),
            ("/docs//", Status::NotFound),
            ("/%zz", Status::BadRequest),
            ("/index.html/extra", Status::NotFound),
        ] {
            assert_eq!(files.resolve(path), Err(status), "{path}");
        }
        #[cfg(unix)]
        for path in ["/link-out", "/link-dir/leak.txt"] {
            assert_eq!(files.resolve(path), Err(Status::Forbidden));
        }
    }
}
