//! Byte ranges are parsed independently of any file or transport.
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
    From(u64),
    FromTo(u64, u64),
    Suffix(u64),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeError {
    Invalid,
    Unsupported,
}
impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid byte range",
            Self::Unsupported => "unsupported range unit or multiple ranges",
        })
    }
}
impl std::error::Error for RangeError {}
impl FromStr for ByteRange {
    type Err = RangeError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (unit, value) = text
            .trim_matches([' ', '\t'])
            .split_once('=')
            .ok_or(RangeError::Invalid)?;
        if !unit.eq_ignore_ascii_case("bytes") {
            return Err(RangeError::Unsupported);
        }
        if value.contains(',') {
            return Err(RangeError::Unsupported);
        }
        let (start, end) = value.split_once('-').ok_or(RangeError::Invalid)?;
        fn number(s: &str) -> Result<u64, RangeError> {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return Err(RangeError::Invalid);
            }
            s.parse().map_err(|_| RangeError::Invalid)
        }
        match (start.is_empty(), end.is_empty()) {
            (true, true) => Err(RangeError::Invalid),
            (true, false) => Ok(Self::Suffix(number(end)?)),
            (false, true) => Ok(Self::From(number(start)?)),
            (false, false) => {
                let start = number(start)?;
                let end = number(end)?;
                if end < start {
                    return Err(RangeError::Invalid);
                }
                Ok(Self::FromTo(start, end))
            }
        }
    }
}
/// Inclusive end on the wire; bounded offset + length for I/O.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedRange {
    pub start: u64,
    pub end: u64,
    pub len: u64,
}
impl ByteRange {
    pub fn resolve(self, total: u64) -> Option<ResolvedRange> {
        let last = total.checked_sub(1)?;
        let (start, end) = match self {
            Self::From(start) => (start, last),
            Self::FromTo(start, end) => (start, end.min(last)),
            Self::Suffix(0) => return None,
            Self::Suffix(n) => (total.saturating_sub(n), last),
        };
        if start > last || end < start {
            return None;
        }
        let len = end.checked_sub(start)?.checked_add(1)?;
        Some(ResolvedRange { start, end, len })
    }
}
