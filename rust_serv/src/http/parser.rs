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
pub fn parse_request(buf: &[u8],) -> Result<Option<(Request, usize)>, ParseError> {
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
    let head_end = match buf.windows(4).position(|window| {window == b"\r\n\r\n" }) {
        Some(pos) => pos,None => {
            if buf.len() > MAX_HEAD {
                return Err(ParseError::HeadersTooLarge);
            }
            return Ok(None);
        }
    };

    // Calculate the length of the head, which is the position of the end of the head plus 4 bytes for the "\r\n\r\n" sequence.
    let head_len = head_end + 4;

    // Convert the head portion of the buffer to a UTF-8 string. If the conversion fails, return a MalformedRequestLine error.
    let head = match std::str::from_utf8(&buf[..head_end]) {
        Ok(head) => head,
        Err(_) => return Err(ParseError::MalformedRequestLine),
    };

    // Split the head into lines using "\r\n" as the delimiter.
    let mut lines = head.split("\r\n");

    // Retrieve the request line (the first line of the HTTP request) from the lines iterator. If there is no request line, 
    // return a MalformedRequestLine error.
    let request_line = match lines.next() {
        Some(line) => line,
        None => return Err(ParseError::MalformedRequestLine),
    };

    // 2. Parse the request line, which should consist of the HTTP method, target, and version separated by spaces.
    let mut parts = request_line.split(' ');
    
    // Extract the method, target, and version from the request line.
    let method_str = parts.next();
    // Extract the target and version from the request line.
    let target = parts.next();
    // Extract the version from the request line.
    let version_str = parts.next();

    
    // Check if any of the method, target, or version are missing, or if there are extra parts in the request line. 
    // If so, return a MalformedRequestLine error.
    if method_str.is_none() || target.is_none() || version_str.is_none() || parts.next().is_some(){
        return Err(ParseError::MalformedRequestLine);
    }

    // Unwrap the method, target, and version strings since we have already checked that they are not None.
    let method_str = method_str.unwrap();
    let target = target.unwrap();
    let version_str = version_str.unwrap();

    // Validate the target to ensure it is not empty and starts with a '/'. If not, return a MalformedRequestLine error.
    if target.is_empty() || !target.starts_with('/') {
        return Err(ParseError::MalformedRequestLine);
    }

    // Validate the method to ensure it is a valid HTTP method. If not, return an UnsupportedMethod error.
    let method = match Method::from_str(method_str) {
        Ok(method) => method,
        Err(_) => return Err(ParseError::UnsupportedMethod),
    };

    // Validate the version to ensure it is a valid HTTP version. If not, return an UnsupportedVersion error.
    let version = match Version::from_str(version_str) {
        Ok(version) => version,
        Err(_) => return Err(ParseError::UnsupportedVersion),
    };

    // 3. Parse the headers, which are the remaining lines after the request line.

    // Create a new Headers struct to store the parsed headers.
    let mut headers = Headers::new();

    // Iterate over the remaining lines to parse each header.
    for line in lines {
        // if line starts with space or tab, it's a continuation of the previous header, which is not supported in this implementation.
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(ParseError::MalformedHeader);
        }

        // Split the first occurrence of ':' to separate the header name and value. If there is no ':', return a MalformedHeader error.
        let (name, value) = match line.split_once(':') {
            Some(pair) => pair,
            None => return Err(ParseError::MalformedHeader),
        };

        // Validate the header name to ensure it is not empty and does not end with a space or tab. If it is invalid, return a MalformedHeader error.
        if name.is_empty() {
            return Err(ParseError::MalformedHeader);
        }

        // Validate the header name to ensure it does not end with a space or tab. If it does, return a MalformedHeader error.
        if name.ends_with(' ') || name.ends_with('\t') {
            return Err(ParseError::MalformedHeader);
        }

        // Trim leading and trailing whitespace from the header value.
        let value = value.trim_matches([' ', '\t']);

        // Add the parsed header name and value to the Headers struct.
        headers.push(name.to_string(), value.to_string());
    }

    // 4 Host

    // If the HTTP version is 1.1, the Host header is required. If it is missing, return a MissingHost error.
    if matches!(version, Version::Http11) && headers.get("Host").is_none(){
        return Err(ParseError::MissingHost);
    }

    // 5. Transfer-Encoding
    if headers.get("Transfer-Encoding").is_some() {
        return Err(ParseError::UnsupportedTransferEncoding);
    }

    // 6. Content-Length
    let mut content_length = 0usize;

    /*
     * Check if the Content-Length header is present. If it is, validate its value and ensure there is only one Content-Length header. 
     * If the value is invalid or there are multiple headers, return an InvalidContentLength error. If the content length exceeds the 
     * maximum allowed body size, return a BodyTooLarge error.
     */
    if let Some(value) = headers.get("Content-Length") {

        // Count the number of Content-Length headers present in the headers. If there is more than one, return an InvalidContentLength error.
        let count = headers
            .iter()
            .filter(|(name, _)| { name.eq_ignore_ascii_case("Content-Length") })
            .count();

        // If there is not exactly one Content-Length header, return an InvalidContentLength error.
        if count != 1 {
            return Err(ParseError::InvalidContentLength);
        }

        // Parse the value of the Content-Length header to determine the length of the body. 
        // If the value is invalid, return an InvalidContentLength error.
        content_length = match value.parse::<usize>() {
            Ok(value) => value,
            Err(_) => return Err(ParseError::InvalidContentLength),
        };

        // If the content length exceeds the maximum allowed body size, return a BodyTooLarge error.
        if content_length > MAX_BODY {
            return Err(ParseError::BodyTooLarge);
        }
    }

    // 7 check if the buffer contains enough bytes to include the entire body based on the Content-Length header.
    let consumed = match head_len.checked_add(content_length) {
        Some(value) => value,
        None => return Err(ParseError::BodyTooLarge),
    };

    // If the buffer does not contain enough bytes to include the entire body, return Ok(None) to indicate that the request is incomplete.
    if buf.len() < consumed {
        return Ok(None);
    }

    // 8 Create a new Request struct with the parsed method, target, version, headers, and body.
    // The body is extracted from the buffer based on the calculated consumed length.
    let body = buf[head_len..consumed].to_vec();

    // Create a new Request struct with the parsed method, target, version, headers, and body.
    let request = Request {
        method,
        target: target.to_string(),
        version,
        headers,
        body,
    };

    Ok(Some((request, consumed)))
}