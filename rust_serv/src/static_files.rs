//! Static files beneath a canonical root. Local filesystem writers must be trusted:
//! canonicalize followed by open still has a TOCTOU window (openat2 can close it).
use crate::http::{
    borrowed_request::Request as BorrowedRequest,
    date::{format_http_date, parse_http_date},
    range::{ByteRange, RangeError},
    request::{Method, Request},
    response::{Body, Response, Status},
    uri::decode_path,
};
use std::{
    fs::{self, File},
    io::{self},
    path::{Path, PathBuf},
};
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
        self.serve_borrowed(&BorrowedRequest::from(request))
    }
    pub fn serve_borrowed(&self, request: &BorrowedRequest<'_>) -> Response {
        self.serve_inner(request)
            .unwrap_or_else(|status| Response::text(status, status.reason()))
            .header("X-Content-Type-Options", "nosniff")
    }
    fn serve_inner(&self, request: &BorrowedRequest<'_>) -> Result<Response, Status> {
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
        let modified = metadata.modified().map_err(io_status)?;
        let last_modified = format_http_date(modified);
        let total = metadata.len();
        // Educational validator: trusts local writers to change mtime on edits.
        // Nanoseconds avoid collisions caused by HTTP-date's whole seconds.
        let stamp = match modified.duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => format!("p{:x}-{:x}", d.as_secs(), d.subsec_nanos()),
            Err(e) => {
                let d = e.duration();
                format!("n{:x}-{:x}", d.as_secs(), d.subsec_nanos())
            }
        };
        let etag = format!("\"{stamp}-{total:x}\"");
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
        .header("Cache-Control", "no-cache")
        .header("Accept-Ranges", "bytes")
        .header("ETag", etag.clone());
        if unmodified {
            return Ok(response);
        }
        let mut response = response;
        let mut offset = 0;
        let mut len = total;
        // Range only applies to GET; HEAD describes the complete representation.
        let ranges: Vec<_> = request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("Range"))
            .collect();
        let validators: Vec<_> = request
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("If-Range"))
            .collect();
        let validator_matches =
            validators.is_empty() || (validators.len() == 1 && validators[0].1 == etag);
        if request.method == Method::Get && ranges.len() == 1 && validator_matches {
            match ranges[0].1.parse::<ByteRange>() {
                Err(RangeError::Unsupported) => {} // Ignore unknown units / multipart ranges.
                parsed => {
                    let resolved = parsed.ok().and_then(|range| range.resolve(total));
                    let Some(range) = resolved else {
                        response.status = Status::RangeNotSatisfiable;
                        return Ok(response.header("Content-Range", format!("bytes */{total}")));
                    };
                    response.status = Status::PartialContent;
                    response = response.header(
                        "Content-Range",
                        format!("bytes {}-{}/{total}", range.start, range.end),
                    );
                    offset = range.start;
                    len = range.len;
                }
            }
        }
        Ok(response.body(Body::File { file, len, offset }))
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
