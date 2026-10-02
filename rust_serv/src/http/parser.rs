use std::str::FromStr;

use crate::http::request::{Headers, Method, Request, Version};

/// Maximum size of the request head (request line + headers + final CRLF CRLF).
pub const MAX_HEAD: usize = 8 * 1024;
/// Maximum accepted body size (Content-Length).
pub const MAX_BODY: usize = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum ParseError {
    MalformedRequestLine,
    UnsupportedMethod,
    UnsupportedVersion,
    MalformedHeader,
    HeadersTooLarge,
    MissingHost,
    InvalidContentLength,
    BodyTooLarge,
    UnsupportedTransferEncoding,
}

/// Parses one HTTP request from the start of `buf`.
///
/// - `Ok(None)`: the request is incomplete; read more bytes and call again.
/// - `Ok(Some((request, consumed)))`: a full request that used the first `consumed` bytes.
/// - `Err(_)`: the request is invalid and cannot be recovered.
///
/// The result depends only on the bytes in `buf`, never on how they were split across reads.
pub fn parse_request(buf: &[u8]) -> Result<Option<(Request, usize)>, ParseError> {
    // The head ends at the first "\r\n\r\n". Without it the request is either
    // incomplete or already too large to ever be accepted.
    let head_end = match buf.windows(4).position(|window| window == b"\r\n\r\n") {
        Some(pos) => pos,
        None => {
            if buf.len() > MAX_HEAD {
                return Err(ParseError::HeadersTooLarge);
            }
            return Ok(None);
        }
    };

    let head_len = head_end + 4;

    // Checked even when the terminator was found, so the result does not depend on read boundaries.
    if head_len > MAX_HEAD {
        return Err(ParseError::HeadersTooLarge);
    }

    // Only the head is converted to text: the body may contain arbitrary bytes.
    let head = std::str::from_utf8(&buf[..head_end]).map_err(|_| ParseError::MalformedHeader)?;

    // Strict on purpose: split on "\r\n" (not `lines()`, which also accepts a bare "\n").
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or(ParseError::MalformedRequestLine)?;

    let (method, target, version) = parse_request_line(request_line)?;
    let headers = parse_headers(lines)?;
    validate_host(version, &headers)?;

    // Transfer-Encoding is checked before Content-Length: the combination of both
    // is the classic request smuggling setup, so it must fail with the most specific error.
    if headers.get("Transfer-Encoding").is_some() {
        return Err(ParseError::UnsupportedTransferEncoding);
    }

    let content_length = parse_content_length(&headers)?;

    // Rejected before waiting for the body, so a huge Content-Length never makes us buffer.
    if content_length > MAX_BODY {
        return Err(ParseError::BodyTooLarge);
    }

    let consumed = head_len
        .checked_add(content_length)
        .ok_or(ParseError::BodyTooLarge)?;

    if buf.len() < consumed {
        return Ok(None);
    }

    let body = buf[head_len..consumed].to_vec();

    let request = Request {
        method,
        target,
        version,
        headers,
        body,
    };

    Ok(Some((request, consumed)))
}

fn parse_request_line(line: &str) -> Result<(Method, String, Version), ParseError> {
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

    Ok((method, target.to_string(), version))
}

/// Origin-form ("/path?query") for every method, plus asterisk-form ("*") for OPTIONS.
/// Every byte must be visible ASCII: anything else must be percent-encoded.
/// This keeps control characters out of logs and out of future responses (injection).
fn is_valid_target(target: &str, method: Method) -> bool {
    let origin_form = target.starts_with('/');
    let asterisk_form = method == Method::Options && target == "*";

    (origin_form || asterisk_form) && target.bytes().all(|byte| byte.is_ascii_graphic())
}

fn parse_headers<'a>(lines: impl Iterator<Item = &'a str>) -> Result<Headers, ParseError> {
    let mut headers = Headers::new();

    for line in lines {
        // Obsolete line folding is not supported (and is a known smuggling vector).
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(ParseError::MalformedHeader);
        }

        let (name, value) = line.split_once(':').ok_or(ParseError::MalformedHeader)?;

        validate_header_name(name)?;
        validate_header_value(value)?;

        let value = value.trim_matches([' ', '\t']);

        headers.push(name.to_string(), value.to_string());
    }

    Ok(headers)
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
fn validate_host(version: Version, headers: &Headers) -> Result<(), ParseError> {
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

fn parse_content_length(headers: &Headers) -> Result<usize, ParseError> {
    let mut values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
        .map(|(_, value)| value.as_str());

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
    value
        .parse::<usize>()
        .map_err(|_| ParseError::BodyTooLarge)
}
