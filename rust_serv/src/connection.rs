//! Buffered requests, time limits, and bounded TCP connection teardown.

use std::io::{self, Read};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::http::parser::parse_request;
use crate::http::request::Request;
use crate::http::response::Response;

const BUFFER_SIZE: usize = 1024;
const LINGER_BYTES: usize = 64 * 1024;
const LINGER_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub idle_timeout: Duration,
    pub request_timeout: Duration,
    pub write_timeout: Duration,
    pub max_requests: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(10),
            max_requests: 100,
        }
    }
}

impl Config {
    pub(crate) fn validate(self) -> io::Result<()> {
        if self.idle_timeout.is_zero()
            || self.request_timeout.is_zero()
            || self.write_timeout.is_zero()
            || self.max_requests == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "timeouts and max_requests must be greater than zero",
            ));
        }
        Ok(())
    }
}

pub struct Connection {
    stream: TcpStream,
    buf: Vec<u8>,
    config: Config,
    // With parser-first reads, any remainder after a completed request came
    // from the latest socket read. Retain its arrival time across handlers.
    last_read_at: Option<Instant>,
}

impl Connection {
    pub fn new(stream: TcpStream, config: Config) -> io::Result<Self> {
        config.validate()?;
        stream.set_write_timeout(Some(config.write_timeout))?;
        Ok(Self {
            stream,
            buf: Vec::with_capacity(BUFFER_SIZE),
            config,
            last_read_at: None,
        })
    }

    pub fn read_request(&mut self) -> Result<Option<Request>, Error> {
        let mut started = if self.buf.is_empty() {
            None
        } else {
            self.last_read_at
        };
        let idle_started = Instant::now();
        loop {
            // Pipelined requests must be consumed before waiting for more bytes.
            if let Some((request, consumed)) = parse_request(&self.buf)? {
                self.buf.drain(..consumed);
                return Ok(Some(request));
            }
            let (remaining, timeout_error) = match started {
                Some(start) => (
                    self.config.request_timeout.saturating_sub(start.elapsed()),
                    Error::RequestTimeout,
                ),
                None => (
                    self.config
                        .idle_timeout
                        .saturating_sub(idle_started.elapsed()),
                    Error::IdleTimeout,
                ),
            };
            if remaining.is_zero() {
                return Err(timeout_error);
            }
            self.stream.set_read_timeout(Some(remaining))?;
            let mut chunk = [0; BUFFER_SIZE];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return if self.buf.is_empty() {
                        Ok(None)
                    } else {
                        Err(Error::UnexpectedEof)
                    };
                }
                Ok(n) => {
                    let now = Instant::now();
                    started.get_or_insert(now);
                    self.last_read_at = Some(now);
                    self.buf.extend_from_slice(&chunk[..n]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(timeout_error);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Bytes already read by this connection belong to an admitted pipelined request.
    pub(crate) fn has_pending_request(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Used when parsing failed before a Request could be constructed.
    pub(crate) fn pending_is_head(&self) -> bool {
        self.buf.starts_with(b"HEAD ")
    }

    pub fn write_response(&mut self, response: &Response, head_only: bool) -> io::Result<()> {
        if head_only {
            response.write_head_to(&mut self.stream)
        } else {
            response.write_to(&mut self.stream)
        }
    }

    /// Sends FIN, then drains unread input for at most 64 KiB or one second.
    /// Best effort: teardown errors do not replace the response's write result.
    pub fn linger_close(&mut self) {
        if self.stream.shutdown(Shutdown::Write).is_err() {
            return;
        }
        // These bytes were already read from the kernel and need no socket drain.
        self.buf.clear();
        let started = Instant::now();
        let mut drained = 0;
        let mut chunk = [0; BUFFER_SIZE];
        while drained < LINGER_BYTES {
            let remaining = LINGER_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() || self.stream.set_read_timeout(Some(remaining)).is_err() {
                break;
            }
            let limit = chunk.len().min(LINGER_BYTES - drained);
            match self.stream.read(&mut chunk[..limit]) {
                Ok(0) => break,
                Ok(n) => drained += n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    }
}
