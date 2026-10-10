//! One networking thread, readiness-driven sockets and bounded work per turn.
use crate::{
    connection::Config,
    epoll::Epoll,
    http::{
        parser::parse_request,
        request::{Method, Version},
        response::{Response, Status},
    },
    router::Router,
    server::{ShutdownHandle, status_for},
    stats::{Stats, Ticket},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::fd::{AsFd, AsRawFd, RawFd},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

/// Each variant owns precisely the progress needed to resume that operation.
#[derive(Debug)]
pub enum ConnectionState {
    Reading,
    Writing {
        bytes: Vec<u8>,
        offset: usize,
        close_after: bool,
        started: Instant,
    },
    Closing {
        deadline: Instant,
        drained: usize,
    },
}
struct Connection {
    stream: TcpStream,
    token: u64,
    input: Vec<u8>,
    state: ConnectionState,
    last_read: Instant,
    request_started: Option<Instant>,
    handled: usize,
    interest: u32,
    _ticket: Ticket,
}
// Fairness without losing ET readiness: unfinished work goes on a user-space
// queue, so hitting this budget never requires another kernel edge.
const TURN_BUDGET: usize = 64;
const TICK_MS: i32 = 20;
const LISTENER: u64 = 0;

impl Connection {
    fn flags(&self, edge: bool) -> u32 {
        let direction = match self.state {
            ConnectionState::Writing { .. } => libc::EPOLLOUT,
            _ => libc::EPOLLIN,
        };
        (direction
            | if direction == libc::EPOLLIN {
                libc::EPOLLRDHUP
            } else {
                0
            }
            | if edge { libc::EPOLLET } else { 0 }) as u32
    }
    fn respond(
        &mut self,
        mut response: Response,
        head: bool,
        close: bool,
        version: Version,
    ) -> io::Result<()> {
        if close {
            response = response.header("Connection", "close");
        } else if version == Version::Http10 {
            response = response.header("Connection", "keep-alive");
        }
        let mut bytes = Vec::new();
        if head {
            response.write_head_to(&mut bytes)?;
        } else {
            response.write_to(&mut bytes)?;
        }
        self.state = ConnectionState::Writing {
            bytes,
            offset: 0,
            close_after: close,
            started: Instant::now(),
        };
        Ok(())
    }
    /// Returns (still alive, needs another immediate turn).
    fn drive(
        &mut self,
        router: &Router,
        config: Config,
        stats: &Stats,
        shutdown: bool,
    ) -> io::Result<(bool, bool)> {
        for _ in 0..TURN_BUDGET {
            match &mut self.state {
                ConnectionState::Reading => {
                    // Parse first: a pipelined request may already be in user memory.
                    match parse_request(&self.input) {
                        Ok(Some((request, consumed))) => {
                            self.input.drain(..consumed);
                            self.request_started = if self.input.is_empty() {
                                None
                            } else {
                                Some(self.last_read)
                            };
                            self.handled += 1;
                            stats.requests.fetch_add(1, Ordering::Relaxed);
                            let response = match catch_unwind(AssertUnwindSafe(|| {
                                router.handle(&request)
                            })) {
                                Ok(response) => response,
                                Err(_) => {
                                    stats.panics.fetch_add(1, Ordering::Relaxed);
                                    Response::text(Status::InternalServerError, "handler panicked")
                                }
                            };
                            let close = !request.keep_alive()
                                || self.handled >= config.max_requests
                                || (shutdown && self.input.is_empty())
                                || matches!(
                                    response.status,
                                    Status::BadRequest
                                        | Status::RequestTimeout
                                        | Status::PayloadTooLarge
                                        | Status::RequestHeaderFieldsTooLarge
                                );
                            self.respond(
                                response,
                                request.method == Method::Head,
                                close,
                                request.version,
                            )?;
                            continue;
                        }
                        Err(error) => {
                            self.respond(
                                Response::text(status_for(&error), &error.to_string()),
                                self.input.starts_with(b"HEAD "),
                                true,
                                Version::Http11,
                            )?;
                            continue;
                        }
                        Ok(None) => {}
                    }
                    let now = Instant::now();
                    if self
                        .request_started
                        .is_some_and(|start| now.duration_since(start) >= config.request_timeout)
                    {
                        self.respond(
                            Response::text(Status::RequestTimeout, "request timeout"),
                            self.input.starts_with(b"HEAD "),
                            true,
                            Version::Http11,
                        )?;
                        continue;
                    }
                    if self.input.is_empty()
                        && (shutdown || now.duration_since(self.last_read) >= config.idle_timeout)
                    {
                        return Ok((false, false));
                    }
                    let mut chunk = [0u8; 8192];
                    match self.stream.read(&mut chunk) {
                        Ok(0) if self.input.is_empty() => return Ok((false, false)),
                        Ok(0) => {
                            self.respond(
                                Response::text(Status::BadRequest, "incomplete request"),
                                self.input.starts_with(b"HEAD "),
                                true,
                                Version::Http11,
                            )?;
                        }
                        Ok(n) => {
                            let now = Instant::now();
                            self.request_started.get_or_insert(now);
                            self.last_read = now;
                            self.input.extend_from_slice(&chunk[..n]);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            return Ok((true, false));
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
                ConnectionState::Writing {
                    bytes,
                    offset,
                    close_after,
                    started,
                } => {
                    if started.elapsed() >= config.write_timeout {
                        return Ok((false, false));
                    }
                    match self.stream.write(&bytes[*offset..]) {
                        Ok(0) => {
                            return Err(io::Error::new(
                                io::ErrorKind::WriteZero,
                                "socket wrote zero bytes",
                            ));
                        }
                        Ok(n) => *offset += n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            return Ok((true, false));
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                    if *offset == bytes.len() {
                        stats.responses.fetch_add(1, Ordering::Relaxed);
                        if *close_after {
                            self.stream.shutdown(Shutdown::Write)?;
                            self.input.clear();
                            self.state = ConnectionState::Closing {
                                deadline: Instant::now() + Duration::from_secs(1),
                                drained: 0,
                            };
                        } else {
                            if self.input.is_empty() {
                                self.last_read = Instant::now();
                            }
                            self.state = ConnectionState::Reading;
                        }
                        // Explicitly resume reading after writing, including in ET.
                    }
                }
                ConnectionState::Closing { deadline, drained } => {
                    if Instant::now() >= *deadline || *drained >= 64 * 1024 {
                        return Ok((false, false));
                    }
                    let mut chunk = [0u8; 8192];
                    match self.stream.read(&mut chunk) {
                        Ok(0) => return Ok((false, false)),
                        Ok(n) => *drained += n,
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            return Ok((true, false));
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return Err(e),
                    }
                }
            }
        }
        Ok((true, true))
    }
    fn needs_tick(&self, config: Config, shutdown: bool) -> bool {
        match &self.state {
            ConnectionState::Reading => {
                (shutdown && self.input.is_empty())
                    || self
                        .request_started
                        .is_some_and(|t| t.elapsed() >= config.request_timeout)
                    || (self.input.is_empty() && self.last_read.elapsed() >= config.idle_timeout)
            }
            ConnectionState::Writing { started, .. } => started.elapsed() >= config.write_timeout,
            ConnectionState::Closing { deadline, .. } => Instant::now() >= *deadline,
        }
    }
}

pub(crate) fn run(
    listener: &TcpListener,
    router: &Router,
    config: Config,
    stats: &Arc<Stats>,
    shutdown: &ShutdownHandle,
    edge: bool,
) -> io::Result<()> {
    let epoll = Epoll::new()?;
    let listener_flags = (libc::EPOLLIN | if edge { libc::EPOLLET } else { 0 }) as u32;
    epoll.add(listener.as_fd(), LISTENER, listener_flags)?;
    let mut connections: HashMap<RawFd, Connection> = HashMap::new();
    // Tokens distinguish different sockets even when the kernel reuses an fd.
    let mut tokens: HashMap<u64, RawFd> = HashMap::new();
    let mut next_token = 1u64;
    let mut ready = VecDeque::new();
    let mut queued = HashSet::new();
    let mut accepting = true;
    let mut listener_registered = true;
    let mut accept_pending = false;
    let mut accept_retry = Instant::now();
    let mut next_tick = Instant::now();
    loop {
        let stopping = shutdown.is_requested();
        if stopping && accepting {
            if listener_registered {
                epoll.delete(listener.as_fd())?;
                listener_registered = false;
            }
            accepting = false;
            accept_pending = false;
        }
        if stopping && connections.is_empty() {
            break;
        }
        if accepting && !listener_registered && Instant::now() >= accept_retry {
            epoll.add(listener.as_fd(), LISTENER, listener_flags)?;
            listener_registered = true;
        }
        let events = match epoll.wait(
            if ready.is_empty() && (!accept_pending || Instant::now() < accept_retry) {
                TICK_MS
            } else {
                0
            },
        ) {
            Ok(events) => events,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        for event in events {
            if event.token == LISTENER {
                if accepting {
                    accept_pending = true;
                }
                continue;
            }
            if let Some(&fd) = tokens.get(&event.token) {
                if event.flags & libc::EPOLLERR as u32 != 0 {
                    match connections[&fd].stream.take_error() {
                        Ok(None) => {}
                        Ok(Some(error)) | Err(error) => {
                            stats.errors.fetch_add(1, Ordering::Relaxed);
                            eprintln!("epoll socket error: {error}");
                            remove(fd, &epoll, &mut connections, &mut tokens);
                            continue;
                        }
                    }
                }
                // RDHUP/HUP do not discard buffered bytes; read drains to EOF.
                enqueue(event.token, &mut ready, &mut queued);
            }
        }
        if accepting && listener_registered && accept_pending && Instant::now() >= accept_retry {
            for _ in 0..TURN_BUDGET {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let ticket = Ticket::new(Arc::clone(stats));
                        if let Err(e) = stream.set_nonblocking(true) {
                            stats.errors.fetch_add(1, Ordering::Relaxed);
                            eprintln!("stream setup: {e}");
                            continue;
                        }
                        let fd = stream.as_raw_fd();
                        let token = next_token;
                        next_token = next_token
                            .checked_add(1)
                            .ok_or_else(|| io::Error::other("epoll token space exhausted"))?;
                        let mut connection = Connection {
                            stream,
                            token,
                            input: Vec::new(),
                            state: ConnectionState::Reading,
                            last_read: Instant::now(),
                            request_started: None,
                            handled: 0,
                            interest: 0,
                            _ticket: ticket,
                        };
                        connection.interest = connection.flags(edge);
                        if let Err(e) =
                            epoll.add(connection.stream.as_fd(), token, connection.interest)
                        {
                            stats.errors.fetch_add(1, Ordering::Relaxed);
                            eprintln!("epoll admission: {e}");
                            continue;
                        }
                        // Ticket is moved into Connection before admission; now start it.
                        connection._ticket.start();
                        connections.insert(fd, connection);
                        tokens.insert(token, fd);
                        enqueue(token, &mut ready, &mut queued);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        accept_pending = false;
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        eprintln!("epoll accept (retry in 50 ms): {e}");
                        accept_retry = Instant::now() + Duration::from_millis(50);
                        epoll.delete(listener.as_fd())?;
                        listener_registered = false;
                        // Keep a user-space retry, since ET may not deliver another edge.
                        break;
                    }
                }
            }
        }
        if Instant::now() >= next_tick {
            for connection in connections.values() {
                if connection.needs_tick(config, stopping) {
                    enqueue(connection.token, &mut ready, &mut queued);
                }
            }
            next_tick = Instant::now() + Duration::from_millis(TICK_MS as u64);
        }
        let turns = ready.len().min(256);
        let mut dead = Vec::new();
        for _ in 0..turns {
            let Some(token) = ready.pop_front() else {
                break;
            };
            queued.remove(&token);
            let Some(&fd) = tokens.get(&token) else {
                continue;
            };
            let Some(connection) = connections.get_mut(&fd) else {
                continue;
            };
            match connection.drive(router, config, stats, stopping) {
                Ok((false, _)) => dead.push(fd),
                Ok((true, again)) => {
                    let flags = connection.flags(edge);
                    if flags != connection.interest {
                        if epoll
                            .modify(connection.stream.as_fd(), token, flags)
                            .is_err()
                        {
                            stats.errors.fetch_add(1, Ordering::Relaxed);
                            dead.push(fd);
                            continue;
                        }
                        connection.interest = flags;
                    }
                    if again {
                        enqueue(token, &mut ready, &mut queued);
                    }
                }
                Err(e) => {
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!("epoll connection: {e}");
                    dead.push(fd);
                }
            }
        }
        // No HashMap removal while a Connection is mutably borrowed above.
        for fd in dead {
            remove(fd, &epoll, &mut connections, &mut tokens);
        }
    }
    Ok(())
}
fn enqueue(token: u64, ready: &mut VecDeque<u64>, queued: &mut HashSet<u64>) {
    if queued.insert(token) {
        ready.push_back(token);
    }
}
fn remove(
    fd: RawFd,
    epoll: &Epoll,
    connections: &mut HashMap<RawFd, Connection>,
    tokens: &mut HashMap<u64, RawFd>,
) {
    if let Some(connection) = connections.remove(&fd) {
        let _ = epoll.delete(connection.stream.as_fd());
        tokens.remove(&connection.token);
        // TcpStream and Ticket close/correct counters by RAII on this scope exit.
    }
}
