use std::str::FromStr;

// Importing necessary types from the request module.
use crate::http::request::{Headers, Method, Request, Version};

// Constants defining maximum sizes for HTTP headers and body.
const MAX_HEAD: usize = 8 * 1024;
const MAX_BODY: usize = 1024 * 1024;

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

// Function to parse an HTTP request from a byte buffer and return a Result containing either an
// Option with the parsed Request and the number of bytes consumed, or a ParseError.
pub fn parse_request(buf: &[u8]) -> Result<Option<(Request, usize)>, ParseError> {
    /*
     * 1. search for the end of the head, which is indicated by the sequence:
     *
     *     \r\n\r\n
     *
     * if the sequence is not found, we have two possibilities:
     *     - buffer <= MAX_HEAD -> request incomplete, return Ok(None)
     *     - buffer > MAX_HEAD  -> headers so large, reject
     */

    // Search for the end of the HTTP request head by looking for the sequence "\r\n\r\n" in the buffer.
    let head_end = match buf.windows(4).position(|window| window == b"\r\n\r\n") {
        Some(pos) => pos,
        None => {
            if buf.len() > MAX_HEAD {
                return Err(ParseError::HeadersTooLarge);
            }
            return Ok(None);
        }
    };

    // Calculate the length of the head, which is the position of the end of the head plus 4 bytes for the "\r\n\r\n" sequence.
    let head_len = head_end + 4;

    // If the length of the head exceeds the maximum allowed size, return a HeadersTooLarge error.
    if head_len > MAX_HEAD {
        return Err(ParseError::HeadersTooLarge);
    }

    // Convert the head portion of the buffer to a UTF-8 string. If the conversion fails, return a MalformedHeader error.
    let head = std::str::from_utf8(&buf[..head_end]).map_err(|_| ParseError::MalformedHeader)?;

    // Split the head into lines using "\r\n" as the delimiter.
    let mut lines = head.split("\r\n");

    // Parse the request line, which is the first line of the HTTP request and contains the method, target, and version.
    let request_line = lines.next().ok_or(ParseError::MalformedRequestLine)?;

    // Parse the request line into its components: method, target, and version.
    let (method, target, version) = parse_request_line(request_line)?;

    //parse the headers from the remaining lines of the HTTP request.
    let headers = parse_headers(lines)?;

    // Validate the presence and correctness of the Host header based on the HTTP version.
    validate_host(&version, &headers)?;

    // Parse the Content-Length header to determine the length of the request body.
    let content_length = parse_content_length(&headers)?;

    // Check if the Transfer-Encoding header is present, which is not supported in this implementation.
    if headers.get("Transfer-Encoding").is_some() {
        return Err(ParseError::UnsupportedTransferEncoding);
    }

    // Check if the content length exceeds the maximum allowed body size. If it does, return a BodyTooLarge error.
    if content_length > MAX_BODY {
        return Err(ParseError::BodyTooLarge);
    }

    // Calculate the total number of bytes that need to be consumed from the buffer, which is the sum of the 
    // head length and the content length.
    let consumed = head_len
        .checked_add(content_length)
        .ok_or(ParseError::BodyTooLarge)?;

    // If the buffer length is less than the total consumed bytes, return Ok(None) to indicate that the request is incomplete.
    if buf.len() < consumed {
        return Ok(None);
    }
    
    // Extract the body of the HTTP request from the buffer, which is the portion of the buffer after the head and up to the consumed length.
    let body = buf[head_len..consumed].to_vec();

    // Create a new Request struct with the parsed method, target, version, headers, and body.
    let request = Request {
        method,
        target,
        version,
        headers,
        body,
    };

    Ok(Some((request, consumed)))
}

// Function to parse the request line of an HTTP request, which consists of the method, target, and version.
fn parse_request_line(line: &str,) -> Result<(Method, String, Version), ParseError> {

    // Split the request line into its components using space as the delimiter.
    let mut parts = line.split(' ');

    // Use pattern matching to extract the method, target, and version from the split parts. 
    // If any of these components are missing, return a MalformedRequestLine error.
    let (Some(method_str), Some(target), Some(version_str), None) = 
    (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(ParseError::MalformedRequestLine);
    };
    
    // Validate that the target is not empty and starts with a forward slash ('/'). 
    // If it does not meet these criteria, return a MalformedRequestLine error.
    if target.is_empty() || !target.starts_with('/') {
        return Err(ParseError::MalformedRequestLine);
    }

    // Convert the method string to a Method enum value. If the conversion fails,
    //  return an UnsupportedMethod error.
    let method = Method::from_str(method_str).map_err(|_| ParseError::UnsupportedMethod)?;

    // Convert the version string to a Version enum value. If the conversion fails,
    //  return an UnsupportedVersion error.
    let version = Version::from_str(version_str).map_err(|_| ParseError::UnsupportedVersion)?;

    // Return the parsed method, target, and version as a tuple.
    Ok((method, target.to_string(), version))
}

// Function to parse the headers of an HTTP request from an iterator of lines.
fn parse_headers<'a, I>(lines: I) -> Result<Headers, ParseError> where I: Iterator<Item = &'a str>, {
    let mut headers = Headers::new();

    // Iterate over each line in the provided iterator of lines.
    for line in lines {
        // Obsolete line folding is not supported.
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(ParseError::MalformedHeader);
        }

        // Split the line into a name and value pair using the first occurrence of ':' as the delimiter.
        let (name, value) = line.split_once(':').ok_or(ParseError::MalformedHeader)?;

        // Validate the header name and value using the respective validation functions.
        validate_header_name(name)?;
        validate_header_value(value)?;

        // Trim leading and trailing whitespace from the header value.
        let value = value.trim_matches([' ', '\t']);

        // Add the validated header name and value to the headers collection.
        headers.push(name.to_string(), value.to_string());
    }
    Ok(headers)
}

// Function to validate the header name of an HTTP request.
fn validate_header_name(name: &str) -> Result<(), ParseError> {
    // Header names must not be empty and must consist of valid token characters.
    if name.is_empty() {
        return Err(ParseError::MalformedHeader);
    }

    // Check if all characters in the header name are valid token characters. 
    // If any character is invalid, return a MalformedHeader error.
    if !name.bytes().all(is_token_char) {
        return Err(ParseError::MalformedHeader);
    }

    Ok(())
}

// Function to check if a byte is a valid token character according to the HTTP specification.
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

// Function to validate the header value of an HTTP request.
fn validate_header_value(value: &str) -> Result<(), ParseError> {
    for byte in value.bytes() {
        // HTAB (0x09) is allowed.
        // Other control characters are rejected.
        if byte < 0x20 && byte != b'\t' {
            return Err(ParseError::MalformedHeader);
        }

        // DEL (0x7f) is also a control character.
        if byte == 0x7f {
            return Err(ParseError::MalformedHeader);
        }
    }

    Ok(())
}

// Function to validate the presence and correctness of the Host header based on the HTTP version.
fn validate_host(version: &Version, headers: &Headers) -> Result<(), ParseError> {
    // Count the number of Host headers in the provided headers collection, ignoring case.
    let host_count = headers.iter().filter(|(name, _)| name.eq_ignore_ascii_case("Host")).count();

    // For HTTP/1.1 requests, the Host header is required and must appear exactly once.
    if *version == Version::Http11 {
        if host_count == 0 {
            return Err(ParseError::MissingHost);
        }

        if host_count > 1 {
            return Err(ParseError::MalformedHeader);
        }
    }
    Ok(())
}

// Function to parse the Content-Length header from the provided headers collection.
fn parse_content_length(headers: &Headers) -> Result<usize, ParseError> {

    // Collect all values of the Content-Length header, ignoring case, into a vector.
    let values: Vec<&str> = headers.iter()
                            .filter(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
                            .map(|(_, value)| value.as_str())
                            .collect();

    // If no Content-Length header is present, return 0 as the default value.
    if values.is_empty() {
        return Ok(0);
    }

    // If there is more than one Content-Length header, return an InvalidContentLength error.
    if values.len() != 1 {
        return Err(ParseError::InvalidContentLength);
    }

    // Extract the single value of the Content-Length header.
    let value = values[0];

    // HTTP Content-Length must contain one or more decimal digits.
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseError::InvalidContentLength);
    }
    // Parse the Content-Length value as a usize. If parsing fails, return an InvalidContentLength error.
    value
        .parse::<usize>()
        .map_err(|_| ParseError::InvalidContentLength)
}