//! Bodies own their resources. A file is never reopened from a request path.
use bytes::Bytes;
use std::{
    fs::File,
    io::{self},
    time::{Duration, Instant},
};

pub const BLOCK_SIZE: usize = 64 * 1024;

#[derive(Debug)]
pub enum Body {
    Empty,
    Bytes(Bytes),
    File {
        file: File,
        len: u64,
        offset: u64,
    },
    /// A finite, lazily generated SSE stream. Its wire length is unknown.
    Stream {
        interval: Duration,
        events: usize,
    },
}
impl Body {
    pub fn len(&self) -> Option<u64> {
        match self {
            Self::Empty => Some(0),
            Self::Bytes(b) => Some(b.len() as u64),
            Self::File { len, .. } => Some(*len),
            Self::Stream { .. } => None,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == Some(0)
    }
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Empty => Some(&[]),
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }
    pub fn cursor(&self) -> io::Result<BodyCursor> {
        Ok(match self {
            Self::Empty => BodyCursor::Empty,
            Self::Bytes(b) => BodyCursor::Bytes(b.clone()),
            Self::File { file, len, offset } => BodyCursor::File {
                file: file.try_clone()?,
                remaining: *len,
                offset: *offset,
            },
            Self::Stream { interval, events } => BodyCursor::Stream {
                interval: *interval,
                events: *events,
                next: 0,
                ready: Instant::now(),
            },
        })
    }
}
impl From<Bytes> for Body {
    fn from(value: Bytes) -> Self {
        Self::Bytes(value)
    }
}
impl From<Vec<u8>> for Body {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value.into())
    }
}
impl From<String> for Body {
    fn from(value: String) -> Self {
        Self::from(value.into_bytes())
    }
}
impl From<&str> for Body {
    fn from(value: &str) -> Self {
        Self::Bytes(Bytes::copy_from_slice(value.as_bytes()))
    }
}
impl From<&[u8]> for Body {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(Bytes::copy_from_slice(value))
    }
}
impl<const N: usize> From<&[u8; N]> for Body {
    fn from(value: &[u8; N]) -> Self {
        Self::from(value.as_slice())
    }
}
impl<T: AsRef<[u8]> + ?Sized> PartialEq<T> for Body {
    fn eq(&self, other: &T) -> bool {
        self.as_bytes() == Some(other.as_ref())
    }
}

#[derive(Debug)]
pub enum BodyCursor {
    Empty,
    Bytes(Bytes),
    File {
        file: File,
        remaining: u64,
        offset: u64,
    },
    Stream {
        interval: Duration,
        events: usize,
        next: usize,
        ready: Instant,
    },
}
#[derive(Debug)]
pub enum Chunk {
    Data(Bytes),
    Pending(Instant),
    Done,
}
impl BodyCursor {
    pub fn next_chunk(&mut self) -> io::Result<Chunk> {
        match self {
            Self::Empty => Ok(Chunk::Done),
            Self::Bytes(bytes) => {
                if bytes.is_empty() {
                    return Ok(Chunk::Done);
                }
                Ok(Chunk::Data(bytes.split_to(bytes.len().min(BLOCK_SIZE))))
            }
            Self::File {
                file,
                remaining,
                offset,
            } => {
                if *remaining == 0 {
                    return Ok(Chunk::Done);
                }
                let mut bytes = vec![0; (*remaining).min(BLOCK_SIZE as u64) as usize];
                let n = loop {
                    match read_at(file, &mut bytes, *offset) {
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        result => break result?,
                    }
                };
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "file shrank during transfer",
                    ));
                }
                *remaining -= n as u64;
                *offset += n as u64;
                bytes.truncate(n);
                Ok(Chunk::Data(bytes.into()))
            }
            Self::Stream {
                interval,
                events,
                next,
                ready,
            } => {
                if *next == *events {
                    return Ok(Chunk::Done);
                }
                if Instant::now() < *ready {
                    return Ok(Chunk::Pending(*ready));
                }
                *next += 1;
                *ready = Instant::now() + *interval;
                Ok(Chunk::Data(
                    format!("id: {next}\ndata: event {next}\n\n").into(),
                ))
            }
        }
    }
}

/// Shared wire encoding. Empty data is not a chunk: zero means end-of-message.
pub fn encode_chunk(bytes: &[u8]) -> Vec<u8> {
    let mut wire = format!("{:X}\r\n", bytes.len()).into_bytes();
    wire.extend_from_slice(bytes);
    wire.extend_from_slice(b"\r\n");
    wire
}

#[cfg(unix)]
fn read_at(file: &mut File, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, bytes, offset)
}
#[cfg(not(unix))]
fn read_at(file: &mut File, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::io::{Read, Seek};
    file.seek(io::SeekFrom::Start(offset))?;
    file.read(bytes)
}
