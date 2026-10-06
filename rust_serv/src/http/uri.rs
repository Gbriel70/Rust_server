//! Percent-decoding shared by path segments and query components.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UriError {
    InvalidEscape,
    InvalidUtf8,
}

impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidEscape => "invalid percent-encoding",
            Self::InvalidUtf8 => "decoded URI is not valid UTF-8",
        })
    }
}

impl std::error::Error for UriError {}

/// Decodes exactly once. `+` is literal; path separator policy belongs to routing.
pub fn percent_decode(input: &str) -> Result<String, UriError> {
    let mut decoded = Vec::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(hex_digit)
                .ok_or(UriError::InvalidEscape)?;
            let low = bytes
                .next()
                .and_then(hex_digit)
                .ok_or(UriError::InvalidEscape)?;
            decoded.push(high * 16 + low);
        } else {
            decoded.push(byte);
        }
    }
    String::from_utf8(decoded).map_err(|_| UriError::InvalidUtf8)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
