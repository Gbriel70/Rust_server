//! Bounded duplex state shared by blocking, readiness and async adapters.
use super::{MAX_OUTPUT, codec::Frame, session::Session};
use crate::{connection::Config, server::ShutdownHandle};
use bytes::{Buf, BytesMut};
use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

const TURN_BUDGET: usize = 32;
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct Connection {
    session: Session,
    input: BytesMut,
    output: Vec<u8>,
    offset: usize,
    last_read: Instant,
    frame_started: Option<Instant>,
    write_started: Instant,
    close_deadline: Option<Instant>,
    finish_after_write: bool,
}
impl Connection {
    pub(crate) fn new(input: BytesMut) -> Self {
        let now = Instant::now();
        let frame_started = (!input.is_empty()).then_some(now);
        Self {
            session: Session::default(),
            input,
            output: Vec::new(),
            offset: 0,
            last_read: now,
            frame_started,
            write_started: now,
            close_deadline: None,
            finish_after_write: false,
        }
    }
    pub(crate) fn wants_write(&self) -> bool {
        self.offset < self.output.len()
    }
    pub(crate) fn is_closing(&self) -> bool {
        self.session.is_closing()
    }
    pub(crate) fn deadline(&self, config: Config) -> Instant {
        if let Some(deadline) = self.close_deadline {
            return deadline;
        }
        if self.wants_write() {
            return self.write_started + config.write_timeout;
        }
        match self.frame_started {
            Some(start) => {
                (start + config.request_timeout).min(self.last_read + config.idle_timeout)
            }
            None => self.last_read + config.idle_timeout,
        }
    }
    fn queue(&mut self, frame: Frame) -> io::Result<()> {
        let encoded = frame.encode();
        if self.output.len() + encoded.len() > MAX_OUTPUT {
            return Err(io::Error::other("WebSocket output limit exceeded"));
        }
        if !self.wants_write() {
            self.write_started = Instant::now();
        }
        self.output.extend_from_slice(&encoded);
        Ok(())
    }
    fn close(&mut self, code: u16, config: Config) -> io::Result<()> {
        if let Some(frame) = self.session.start_close(code) {
            self.queue(frame)?;
            self.close_deadline = Some(Instant::now() + CLOSE_TIMEOUT.min(config.write_timeout));
        }
        Ok(())
    }
    /// Returns (alive, immediate continuation). Each turn has a fixed work budget.
    /// Socket waits and readiness registration belong to the caller.
    pub(crate) fn drive(
        &mut self,
        stream: &mut (impl Read + Write),
        config: Config,
        shutdown: bool,
    ) -> io::Result<(bool, bool)> {
        for _ in 0..TURN_BUDGET {
            let now = Instant::now();
            if self.close_deadline.is_some_and(|deadline| now >= deadline)
                || (self.wants_write() && self.write_started.elapsed() >= config.write_timeout)
            {
                return Ok((false, false));
            }
            if shutdown {
                self.close(1001, config)?;
            }
            if !self.is_closing() && !self.wants_write() && now >= self.deadline(config) {
                self.close(1001, config)?;
            }
            if self.wants_write() {
                match stream.write(&self.output[self.offset..]) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => {
                        self.offset += n;
                        self.write_started = Instant::now();
                    }
                    Err(error) if would_wait(&error) => return Ok((true, false)),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
                if !self.wants_write() {
                    self.output.clear();
                    self.offset = 0;
                    if self.finish_after_write || self.session.peer_closed() {
                        return Ok((false, false));
                    }
                }
                continue;
            }
            let received = match Frame::decode(&self.input) {
                Ok(Some((frame, consumed))) => {
                    self.input.advance(consumed);
                    self.frame_started = (!self.input.is_empty()).then_some(self.last_read);
                    self.session.receive(frame).map(Some)
                }
                Ok(None) => Ok(None),
                Err(error) => Err(error),
            };
            match received {
                Ok(Some(reply)) => {
                    if self.session.peer_closed() {
                        self.finish_after_write = true;
                        self.close_deadline = Some(now + CLOSE_TIMEOUT.min(config.write_timeout));
                    }
                    if let Some(frame) = reply {
                        self.queue(frame)?;
                    }
                    if self.session.peer_closed() && !self.wants_write() {
                        return Ok((false, false));
                    }
                    continue;
                }
                Err(error) => {
                    self.close(error.close_code(), config)?;
                    self.finish_after_write = true;
                    self.input.clear();
                    continue;
                }
                Ok(None) => {}
            }
            let mut chunk = [0; 8192];
            match stream.read(&mut chunk) {
                Ok(0) => return Ok((false, false)),
                Ok(n) => {
                    self.last_read = Instant::now();
                    self.frame_started.get_or_insert(self.last_read);
                    self.input.extend_from_slice(&chunk[..n]);
                }
                Err(error) if would_wait(&error) => return Ok((true, false)),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok((true, true))
    }
}
fn would_wait(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

pub(crate) fn run_blocking(
    mut stream: TcpStream,
    input: BytesMut,
    config: Config,
    shutdown: &ShutdownHandle,
) -> io::Result<()> {
    // A session keeps its worker. Short bounded waits observe shutdown/deadlines.
    let poll = POLL_INTERVAL
        .min(config.write_timeout)
        .min(config.request_timeout)
        .min(config.idle_timeout);
    stream.set_read_timeout(Some(poll))?;
    stream.set_write_timeout(Some(poll))?;
    let mut connection = Connection::new(input);
    while connection
        .drive(&mut stream, config, shutdown.is_requested())?
        .0
    {}
    Ok(())
}

struct AsyncSocket<'a>(&'a tokio::net::TcpStream);
impl Read for AsyncSocket<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.try_read(bytes)
    }
}
impl Write for AsyncSocket<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.try_write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub(crate) async fn run_async(
    stream: tokio::net::TcpStream,
    input: BytesMut,
    config: Config,
    shutdown: &ShutdownHandle,
) -> io::Result<()> {
    let mut connection = Connection::new(input);
    loop {
        let (alive, again) =
            connection.drive(&mut AsyncSocket(&stream), config, shutdown.is_requested())?;
        if !alive {
            return Ok(());
        }
        if again {
            tokio::task::yield_now().await;
            continue;
        }
        let interest = if connection.wants_write() {
            tokio::io::Interest::WRITABLE
        } else {
            tokio::io::Interest::READABLE
        };
        tokio::select! {
            ready = stream.ready(interest) => { ready?; },
            _ = tokio::time::sleep_until(connection.deadline(config).into()) => {},
            _ = shutdown.notified(), if !connection.is_closing() => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websocket::MAX_MESSAGE;

    #[derive(Default)]
    struct Socket {
        incoming: std::collections::VecDeque<u8>,
        written: Vec<u8>,
        blocked: bool,
        reads: usize,
    }
    impl Read for Socket {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if self.incoming.is_empty() {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let length = bytes.len().min(self.incoming.len()).min(3);
            for byte in &mut bytes[..length] {
                *byte = self.incoming.pop_front().unwrap();
            }
            Ok(length)
        }
    }
    impl Write for Socket {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.blocked {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let length = bytes.len().min(3);
            self.written.extend_from_slice(&bytes[..length]);
            Ok(length)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn partial_writes_and_fragmented_reads_preserve_frame_order() {
        // Mask is zero. A binary echo is followed by a valid client close.
        let mut socket = Socket {
            incoming: [
                0x82, 0x83, 0, 0, 0, 0, 1, 2, 3, 0x88, 0x82, 0, 0, 0, 0, 3, 232,
            ]
            .into(),
            ..Socket::default()
        };
        let mut connection = Connection::new(BytesMut::new());
        for _ in 0..100 {
            if !connection
                .drive(&mut socket, Config::default(), false)
                .unwrap()
                .0
            {
                assert_eq!(socket.written, [0x82, 3, 1, 2, 3, 0x88, 2, 3, 232]);
                return;
            }
        }
        panic!("session failed to finish within bounded turns");
    }
    #[test]
    fn output_backpressure_stops_input_and_respects_memory_bound() {
        let mut connection = Connection::new(BytesMut::new());
        connection
            .queue(Frame {
                fin: true,
                opcode: super::super::codec::Opcode::Binary,
                payload: vec![0; MAX_MESSAGE],
            })
            .unwrap();
        let mut socket = Socket {
            blocked: true,
            ..Socket::default()
        };
        assert_eq!(
            connection
                .drive(&mut socket, Config::default(), false)
                .unwrap(),
            (true, false)
        );
        assert_eq!(socket.reads, 0);
        assert!(connection.output.len() <= MAX_OUTPUT);
        // A close may follow a partly written echo, preserving the wire order.
        connection
            .drive(&mut socket, Config::default(), true)
            .unwrap();
        assert!(connection.output.len() <= MAX_OUTPUT);
        assert!(connection.is_closing());
    }
    #[test]
    fn pending_write_and_close_deadlines_release_slow_clients() {
        let mut connection = Connection::new(BytesMut::new());
        connection.queue(Frame::close(1000)).unwrap();
        connection.write_started = Instant::now() - Duration::from_secs(20);
        let mut socket = Socket {
            blocked: true,
            ..Socket::default()
        };
        assert!(
            !connection
                .drive(&mut socket, Config::default(), false)
                .unwrap()
                .0
        );
        let mut connection = Connection::new(BytesMut::new());
        connection.close(1001, Config::default()).unwrap();
        connection.close_deadline = Some(Instant::now() - Duration::from_millis(1));
        assert!(
            !connection
                .drive(&mut socket, Config::default(), true)
                .unwrap()
                .0
        );
    }
}
