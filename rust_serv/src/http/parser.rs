//! Incremental parser for HTTP requests received as bytes.

use super::borrowed_request::{
    HeaderIndex, Headers as BorrowedHeaders, Request as BorrowedRequest,
};
use bytes::Bytes;
use std::{borrow::Cow, ops::Range, str::FromStr};

use crate::http::request::{Method, Request, Version};

/// Maximum size of the request head (request line + headers + final CRLF CRLF).
pub const MAX_HEAD: usize = 8 * 1024;
/// Maximum decoded request body size, for fixed-length and chunked input.
pub const MAX_BODY: usize = 1024 * 1024;

/// Reasons why a request cannot be accepted.
#[derive(Debug, PartialEq)]
pub enum ParseError {
    /// The request line does not contain exactly a method, target, and version.
    MalformedRequestLine,
    /// The provided method is not supported.
    UnsupportedMethod,
    /// The provided HTTP version is not supported.
    UnsupportedVersion,
    /// A header does not follow the required syntax or contains invalid characters.
    MalformedHeader,
    /// The header block exceeds `MAX_HEAD` bytes.
    HeadersTooLarge,
    /// An HTTP/1.1 request does not contain exactly one `Host` header.
    MissingHost,
    /// `Content-Length` is repeated or has an invalid format.
    InvalidContentLength,
    /// The declared body exceeds `MAX_BODY` bytes.
    BodyTooLarge,
    /// The requested transfer coding or coding chain is not implemented.
    UnsupportedTransferEncoding,
    AmbiguousFraming,
    MalformedChunk,
}

/// Converts a parsing error into a message suitable for logs and responses.
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::MalformedRequestLine => "malformed request line",
            Self::UnsupportedMethod => "unsupported method",
            Self::UnsupportedVersion => "unsupported HTTP version",
            Self::MalformedHeader => "malformed header",
            Self::HeadersTooLarge => "request headers too large",
            Self::MissingHost => "missing Host header",
            Self::InvalidContentLength => "invalid Content-Length",
            Self::BodyTooLarge => "request body too large",
            Self::UnsupportedTransferEncoding => "unsupported Transfer-Encoding",
            Self::AmbiguousFraming => "ambiguous message framing",
            Self::MalformedChunk => "malformed chunked body",
        };
        f.write_str(message)
    }
}

impl std::error::Error for ParseError {}

/// Parses one HTTP request from the start of `buf`.
///
/// - `Ok(None)`: the request is incomplete; read more bytes and call again.
/// - `Ok(Some((request, consumed)))`: a full request that used the first `consumed` bytes.
/// - `Err(_)`: the request is invalid and cannot be recovered.
///
/// The result depends only on the bytes in `buf`, never on how they were split across reads.
pub fn parse_request(buf: &[u8]) -> Result<Option<(Request, usize)>, ParseError> {
    RequestParser::default().parse(buf)
}

#[derive(Debug, Default)]
enum BodyState {
    #[default]
    Head,
    Fixed(usize),
    Size,
    Data(usize),
    DataCrlf,
    Trailers,
}
/// Holds progress across reads. The caller retains the unconsumed request prefix
/// until completion; pipelined bytes after `consumed` belong to the next request.
#[derive(Debug, Default)]
pub struct RequestParser {
    request: Option<ParsedRequest>,
    offset: usize,
    framing_bytes: usize,
    state: BodyState,
}
impl RequestParser {
    pub fn parse(&mut self, buf: &[u8]) -> Result<Option<(Request, usize)>, ParseError> {
        self.parse_borrowed(buf)
            .map(|result| result.map(|(request, n)| (request.into_owned_request(), n)))
    }
    pub fn parse_borrowed<'a>(
        &mut self,
        buf: &'a [u8],
    ) -> Result<Option<(BorrowedRequest<'a>, usize)>, ParseError> {
        self.parse_indices(buf)?
            .map(|plan| {
                let n = plan.consumed;
                Ok((plan.into_request(buf)?, n))
            })
            .transpose()
    }
    pub fn parse_indices(&mut self, buf: &[u8]) -> Result<Option<ParsedRequest>, ParseError> {
        // An inconsistent incremental caller cannot make slicing/subtraction panic.
        if buf.len() < self.offset {
            return Err(ParseError::MalformedChunk);
        }
        loop {
            match self.state {
                BodyState::Head => {
                    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                        return if buf.len() > MAX_HEAD {
                            Err(ParseError::HeadersTooLarge)
                        } else {
                            Ok(None)
                        };
                    };
                    if end + 4 > MAX_HEAD {
                        return Err(ParseError::HeadersTooLarge);
                    }
                    let head = std::str::from_utf8(&buf[..end])
                        .map_err(|_| ParseError::MalformedHeader)?;
                    let mut lines = head.split("\r\n");
                    let (method, target, version) =
                        parse_request_line(lines.next().ok_or(ParseError::MalformedRequestLine)?)?;
                    let fields = parse_header_indices(head, lines)?;
                    let headers = BorrowedHeaders::indexed(head, Cow::Borrowed(&fields));
                    validate_host(version, &headers)?;
                    let mut encodings = headers
                        .iter()
                        .filter(|(n, _)| n.eq_ignore_ascii_case("Transfer-Encoding"))
                        .map(|(_, v)| v);
                    let encoding = encodings.next();
                    let repeated_encoding = encodings.next().is_some();
                    if encoding.is_some() && headers.get("Content-Length").is_some() {
                        return Err(ParseError::AmbiguousFraming);
                    }
                    self.state = if encoding.is_some() {
                        if version == Version::Http10 || repeated_encoding {
                            return Err(ParseError::AmbiguousFraming);
                        }
                        if !encoding.is_some_and(|value| value.eq_ignore_ascii_case("chunked")) {
                            return Err(ParseError::UnsupportedTransferEncoding);
                        }
                        BodyState::Size
                    } else {
                        let len = parse_content_length(&headers)?;
                        if len > MAX_BODY {
                            return Err(ParseError::BodyTooLarge);
                        }
                        BodyState::Fixed(len)
                    };
                    self.offset = end + 4;
                    self.request = Some(ParsedRequest {
                        method,
                        target: string_range(head, target),
                        version,
                        fields,
                        head_len: end,
                        body: Vec::new(),
                        fixed_body: None,
                        consumed: 0,
                    });
                }
                BodyState::Fixed(len) => {
                    if buf.len() - self.offset < len {
                        return Ok(None);
                    }
                    let request = self.request.as_mut().ok_or(ParseError::MalformedChunk)?;
                    request.fixed_body = Some(self.offset..self.offset + len);
                    self.offset += len;
                    return self.finish();
                }
                BodyState::Size | BodyState::Trailers => {
                    let tail = &buf[self.offset..];
                    let Some(end) = tail.windows(2).position(|w| w == b"\r\n") else {
                        if self.framing_bytes + tail.len() > MAX_HEAD {
                            return Err(ParseError::HeadersTooLarge);
                        }
                        return Ok(None);
                    };
                    self.framing_bytes += end + 2;
                    if self.framing_bytes > MAX_HEAD {
                        return Err(ParseError::HeadersTooLarge);
                    }
                    let line = std::str::from_utf8(&tail[..end])
                        .map_err(|_| ParseError::MalformedChunk)?;
                    self.offset += end + 2;
                    if matches!(self.state, BodyState::Trailers) {
                        if line.is_empty() {
                            return self.finish();
                        }
                        let trailer_fields = parse_header_indices(line, std::iter::once(line))?;
                        let trailers =
                            BorrowedHeaders::indexed(line, Cow::Borrowed(&trailer_fields));
                        // Do not merge trailers into routing, framing, auth, or representation headers.
                        for (name, _) in trailers.iter() {
                            if !name
                                .get(..2)
                                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("x-"))
                            {
                                return Err(ParseError::MalformedChunk);
                            }
                        }
                    } else {
                        let size = parse_chunk_size(line)?;
                        let current = self
                            .request
                            .as_ref()
                            .ok_or(ParseError::MalformedChunk)?
                            .body
                            .len();
                        if size > MAX_BODY - current {
                            return Err(ParseError::BodyTooLarge);
                        }
                        self.state = if size == 0 {
                            BodyState::Trailers
                        } else {
                            BodyState::Data(size)
                        };
                    }
                }
                BodyState::Data(remaining) => {
                    let available = remaining.min(buf.len() - self.offset);
                    let request = self.request.as_mut().ok_or(ParseError::MalformedChunk)?;
                    request
                        .body
                        .extend_from_slice(&buf[self.offset..self.offset + available]);
                    self.offset += available;
                    self.state = if available == remaining {
                        BodyState::DataCrlf
                    } else {
                        BodyState::Data(remaining - available)
                    };
                    if available < remaining {
                        return Ok(None);
                    }
                }
                BodyState::DataCrlf => {
                    if buf.len() - self.offset < 2 {
                        return Ok(None);
                    }
                    if &buf[self.offset..self.offset + 2] != b"\r\n" {
                        return Err(ParseError::MalformedChunk);
                    }
                    self.offset += 2;
                    self.framing_bytes += 2;
                    self.state = BodyState::Size;
                }
            }
        }
    }
    fn finish(&mut self) -> Result<Option<ParsedRequest>, ParseError> {
        let mut request = self.request.take().ok_or(ParseError::MalformedChunk)?;
        let consumed = self.offset;
        *self = Self::default();
        request.consumed = consumed;
        Ok(Some(request))
    }
}
fn parse_chunk_size(line: &str) -> Result<usize, ParseError> {
    let (digits, extensions) = line.split_once(';').map_or((line, ""), |(a, b)| (a, b));
    let digits = if line.contains(';') {
        digits.trim_end_matches([' ', '\t'])
    } else {
        digits
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ParseError::MalformedChunk);
    }
    if line.contains(';') {
        validate_extensions(extensions)?;
    }
    usize::from_str_radix(digits, 16).map_err(|_| ParseError::BodyTooLarge)
}
fn validate_extensions(mut text: &str) -> Result<(), ParseError> {
    loop {
        text = text.trim_start_matches([' ', '\t']);
        let n = text.bytes().take_while(|b| is_token_char(*b)).count();
        if n == 0 {
            return Err(ParseError::MalformedChunk);
        }
        text = text[n..].trim_start_matches([' ', '\t']);
        if let Some(rest) = text.strip_prefix('=') {
            text = rest.trim_start_matches([' ', '\t']);
            if let Some(rest) = text.strip_prefix('"') {
                let bytes = rest.as_bytes();
                let mut i = 0;
                loop {
                    let Some(&b) = bytes.get(i) else {
                        return Err(ParseError::MalformedChunk);
                    };
                    if b == b'"' {
                        text = &rest[i + 1..];
                        break;
                    }
                    if b == b'\\' {
                        i += 1;
                        let Some(&escaped) = bytes.get(i) else {
                            return Err(ParseError::MalformedChunk);
                        };
                        if !(escaped == b'\t' || (0x20..=0x7e).contains(&escaped)) {
                            return Err(ParseError::MalformedChunk);
                        }
                    } else if !(b == b'\t' || (0x20..=0x7e).contains(&b)) {
                        return Err(ParseError::MalformedChunk);
                    }
                    i += 1;
                }
            } else {
                let n = text.bytes().take_while(|b| is_token_char(*b)).count();
                if n == 0 {
                    return Err(ParseError::MalformedChunk);
                }
                text = &text[n..];
            }
        }
        text = text.trim_start_matches([' ', '\t']);
        if text.is_empty() {
            return Ok(());
        }
        text = text.strip_prefix(';').ok_or(ParseError::MalformedChunk)?;
    }
}

fn parse_request_line(line: &str) -> Result<(Method, &str, Version), ParseError> {
    let mut parts = line.split(' ');

    // Exactly three parts separated by a single space.
    let (Some(method_str), Some(target), Some(version_str), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ParseError::MalformedRequestLine);
    };

    // The method is parsed first because the validity of the target depends on it (OPTIONS *).
    let method = Method::from_str(method_str).map_err(|_| ParseError::UnsupportedMethod)?;

    if !is_valid_target(target, method) {
        return Err(ParseError::MalformedRequestLine);
    }

    let version = Version::from_str(version_str).map_err(|_| ParseError::UnsupportedVersion)?;

    Ok((method, target, version))
}

/// Origin-form ("/path?query") for every method, plus asterisk-form ("*") for OPTIONS.
/// Every byte must be visible ASCII: anything else must be percent-encoded.
/// This keeps control characters out of logs and out of future responses (injection).
fn is_valid_target(target: &str, method: Method) -> bool {
    let origin_form = target.starts_with('/');
    let asterisk_form = method == Method::Options && target == "*";

    (origin_form || asterisk_form) && target.bytes().all(|byte| byte.is_ascii_graphic())
}

fn string_range(head: &str, value: &str) -> Range<usize> {
    let start = value.as_ptr() as usize - head.as_ptr() as usize;
    start..start + value.len()
}
fn parse_header_indices<'a>(
    head: &'a str,
    lines: impl Iterator<Item = &'a str>,
) -> Result<Vec<HeaderIndex>, ParseError> {
    let mut fields = Vec::new();
    for line in lines {
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(ParseError::MalformedHeader);
        }
        let (name, value) = line.split_once(':').ok_or(ParseError::MalformedHeader)?;
        validate_header_name(name)?;
        validate_header_value(value)?;
        fields.push(HeaderIndex {
            name: string_range(head, name),
            value: string_range(head, value.trim_matches([' ', '\t'])),
        });
    }
    Ok(fields)
}

/// A header name is a non-empty token. This also rejects "Host : x" (space before the colon).
fn validate_header_name(name: &str) -> Result<(), ParseError> {
    if name.is_empty() || !name.bytes().all(is_token_char) {
        return Err(ParseError::MalformedHeader);
    }

    Ok(())
}

fn is_token_char(byte: u8) -> bool {
    matches!(
        byte,
        b'a'..=b'z'
            | b'A'..=b'Z'
            | b'0'..=b'9'
            | b'!'
            | b'#'
            | b'$'
            | b'%'
            | b'&'
            | b'\''
            | b'*'
            | b'+'
            | b'-'
            | b'.'
            | b'^'
            | b'_'
            | b'`'
            | b'|'
            | b'~'
    )
}

/// Control characters (except HTAB) are rejected, including bare CR/LF and DEL.
fn validate_header_value(value: &str) -> Result<(), ParseError> {
    for byte in value.bytes() {
        if (byte < 0x20 && byte != b'\t') || byte == 0x7f {
            return Err(ParseError::MalformedHeader);
        }
    }

    Ok(())
}

/// HTTP/1.1 requires exactly one Host header; HTTP/1.0 does not.
fn validate_host(version: Version, headers: &BorrowedHeaders<'_>) -> Result<(), ParseError> {
    let host_count = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Host"))
        .count();

    if version == Version::Http11 {
        if host_count == 0 {
            return Err(ParseError::MissingHost);
        }

        if host_count > 1 {
            return Err(ParseError::MalformedHeader);
        }
    }

    Ok(())
}

fn parse_content_length(headers: &BorrowedHeaders<'_>) -> Result<usize, ParseError> {
    let mut values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
        .map(|(_, value)| value);

    let Some(value) = values.next() else {
        return Ok(0);
    };

    // Repeated Content-Length is rejected even when the values are identical: simpler and safer.
    if values.next().is_some() {
        return Err(ParseError::InvalidContentLength);
    }

    // The grammar is 1*DIGIT. `usize::from_str` would also accept a leading '+'.
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError::InvalidContentLength);
    }

    // At this point the value is only digits, so the only possible failure is overflow:
    // the number is too large to be a valid body size.
    value.parse::<usize>().map_err(|_| ParseError::BodyTooLarge)
}

/// No self-reference: this plan stores offsets, never pointers into its input.
#[derive(Debug)]
pub struct ParsedRequest {
    method: Method,
    target: Range<usize>,
    version: Version,
    fields: Vec<HeaderIndex>,
    head_len: usize,
    body: Vec<u8>,
    fixed_body: Option<Range<usize>>,
    consumed: usize,
}
impl ParsedRequest {
    pub fn consumed(&self) -> usize {
        self.consumed
    }
    /// Use the same unchanged prefix that produced this plan.
    /// Bounds and UTF-8 checks protect against incorrect API usage.
    pub fn request<'a>(&'a self, buf: &'a [u8]) -> Result<BorrowedRequest<'a>, ParseError> {
        let head = self.head(buf)?;
        let target = head
            .get(self.target.clone())
            .ok_or(ParseError::MalformedRequestLine)?;
        let body = match &self.fixed_body {
            Some(range) => buf.get(range.clone()).ok_or(ParseError::MalformedChunk)?,
            None => &self.body,
        };
        Ok(BorrowedRequest {
            method: self.method,
            target: Cow::Borrowed(target),
            version: self.version,
            headers: BorrowedHeaders::indexed(head, Cow::Borrowed(&self.fields)),
            body: Cow::Borrowed(body),
        })
    }
    fn head<'a>(&self, buf: &'a [u8]) -> Result<&'a str, ParseError> {
        let head = std::str::from_utf8(
            buf.get(..self.head_len)
                .ok_or(ParseError::MalformedHeader)?,
        )
        .map_err(|_| ParseError::MalformedHeader)?;
        for field in &self.fields {
            if head.get(field.name.clone()).is_none() || head.get(field.value.clone()).is_none() {
                return Err(ParseError::MalformedHeader);
            }
        }
        Ok(head)
    }
    fn into_request(self, buf: &[u8]) -> Result<BorrowedRequest<'_>, ParseError> {
        let head = self.head(buf)?;
        let target = head
            .get(self.target.clone())
            .ok_or(ParseError::MalformedRequestLine)?;
        let body = match self.fixed_body {
            Some(range) => Cow::Borrowed(buf.get(range).ok_or(ParseError::MalformedChunk)?),
            None => Cow::Owned(self.body),
        };
        Ok(BorrowedRequest {
            method: self.method,
            target: Cow::Borrowed(target),
            version: self.version,
            headers: BorrowedHeaders::indexed(head, Cow::Owned(self.fields)),
            body,
        })
    }
    pub fn freeze(self, wire: Bytes) -> Result<RequestFrame, ParseError> {
        if self.consumed != wire.len() {
            return Err(ParseError::MalformedChunk);
        }
        self.request(&wire)?;
        Ok(RequestFrame { wire, plan: self })
    }
}
/// Bytes + indices can safely move into a 'static task. Views are made on demand.
#[derive(Debug)]
pub struct RequestFrame {
    wire: Bytes,
    plan: ParsedRequest,
}
impl RequestFrame {
    pub fn request(&self) -> Result<BorrowedRequest<'_>, ParseError> {
        self.plan.request(&self.wire)
    }
}
pub fn parse_borrowed(buf: &[u8]) -> Result<Option<(BorrowedRequest<'_>, usize)>, ParseError> {
    RequestParser::default().parse_borrowed(buf)
}
