//! Async transport adapter. HTTP parser/router/response stay sans-I/O.
use crate::{
    connection::Config,
    error::Error,
    http::{
        parser::parse_request,
        request::{Method, Request, Version},
        response::{Response, Status},
    },
    router::Router,
    server::{ShutdownHandle, status_for},
    stats::{Stats, Ticket},
};
use bytes::{Buf, BytesMut};
use std::{
    io,
    net::TcpListener as StdListener,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    task::JoinSet,
    time::{Instant, timeout, timeout_at},
};
use tracing::Instrument;

struct Connection {
    stream: TcpStream,
    input: BytesMut,
    config: Config,
    request_started: Option<Instant>,
    last_read: Instant,
    idle_since: Instant,
}
impl Connection {
    fn new(stream: TcpStream, config: Config) -> Self {
        let now = Instant::now();
        Self {
            stream,
            input: BytesMut::with_capacity(1024),
            config,
            request_started: None,
            last_read: now,
            idle_since: now,
        }
    }
    fn has_pending(&self) -> bool {
        !self.input.is_empty()
    }
    fn pending_is_head(&self) -> bool {
        self.input.starts_with(b"HEAD ")
    }
    /// Before retiring an idle socket on shutdown, admit bytes already ready
    /// in the kernel. This never waits for a new packet or opens a new socket.
    fn admit_ready(&mut self) -> Result<bool, Error> {
        if self.has_pending() {
            return Ok(true);
        }
        let mut chunk = [0; 8192];
        loop {
            match self.stream.try_read(&mut chunk) {
                Ok(0) => return Ok(false),
                Ok(n) => {
                    let now = Instant::now();
                    self.input.extend_from_slice(&chunk[..n]);
                    self.request_started.get_or_insert(now);
                    self.last_read = now;
                    return Ok(true);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }
    /// Cancellation-safe under shutdown select: BytesMut and absolute deadline
    /// live in self, and read_buf does not consume bytes when its branch loses.
    async fn read_request(&mut self) -> Result<Option<Request>, Error> {
        loop {
            if let Some((request, consumed)) = parse_request(&self.input)? {
                self.input.advance(consumed);
                self.request_started = if self.input.is_empty() {
                    None
                } else {
                    Some(self.last_read)
                };
                return Ok(Some(request));
            }
            let (deadline, expired) = match self.request_started {
                Some(start) => (start + self.config.request_timeout, Error::RequestTimeout),
                None => (
                    self.idle_since + self.config.idle_timeout,
                    Error::IdleTimeout,
                ),
            };
            if Instant::now() >= deadline {
                return Err(expired);
            }
            // Limit each read independently of BytesMut's spare capacity.
            let n = timeout_at(
                deadline,
                (&mut self.stream).take(8192).read_buf(&mut self.input),
            )
            .await
            .map_err(|_| expired)??;
            if n == 0 {
                return if self.input.is_empty() {
                    Ok(None)
                } else {
                    Err(Error::UnexpectedEof)
                };
            }
            let now = Instant::now();
            self.request_started.get_or_insert(now);
            self.last_read = now;
        }
    }
    async fn write_response(&mut self, response: &Response, head: bool) -> io::Result<()> {
        let mut bytes = Vec::new();
        if head {
            response.write_head_to(&mut bytes)?;
        } else {
            response.write_to(&mut bytes)?;
        }
        // write_all is not cancellation-safe for reuse. On timeout the caller
        // closes this socket; it never starts another response on the same stream.
        timeout(self.config.write_timeout, self.stream.write_all(&bytes))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "response write timeout"))??;
        self.idle_since = Instant::now();
        Ok(())
    }
    async fn linger_close(&mut self) {
        if self.stream.shutdown().await.is_err() {
            return;
        }
        self.input.clear();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut drained = 0;
        let mut chunk = [0; 1024];
        while drained < 64 * 1024 {
            let limit = chunk.len().min(64 * 1024 - drained);
            match timeout_at(deadline, self.stream.read(&mut chunk[..limit])).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(n)) => drained += n,
                Ok(Err(e)) if e.kind() == io::ErrorKind::Interrupted => continue,
                Ok(Err(_)) => break,
            }
        }
    }
}

pub(crate) async fn run(
    listener: &StdListener,
    router: &Arc<Router>,
    config: Config,
    stats: &Arc<Stats>,
    shutdown: &ShutdownHandle,
    max_connections: usize,
) -> io::Result<()> {
    let listener = TcpListener::from_std(listener.try_clone()?)?;
    let limit = Arc::new(Semaphore::new(max_connections));
    let mut tasks = JoinSet::new();
    let interrupt = tokio::signal::ctrl_c();
    tokio::pin!(interrupt);
    let mut result = Ok(());
    tracing::info!(max_connections, "Tokio accept loop started");
    loop {
        // Acquire before accept: a full semaphore applies backlog backpressure,
        // rather than accepting unlimited tasks that only wait for a permit.
        let permit = tokio::select! {
            biased;
            _=shutdown.notified()=>break,
            signal=&mut interrupt=>{ result=signal;shutdown.request_shutdown();break; },
            joined=tasks.join_next(),if !tasks.is_empty()=>{ report(joined,stats);continue; },
            permit=Arc::clone(&limit).acquire_owned()=>permit.map_err(io::Error::other)?,
        };
        let accepted = tokio::select! {
            biased;
            _=shutdown.notified()=>break,
            signal=&mut interrupt=>{ result=signal;shutdown.request_shutdown();break; },
            joined=tasks.join_next(),if !tasks.is_empty()=>{ report(joined,stats);continue; },
            accepted=listener.accept()=>accepted,
        };
        let (stream, peer) = match accepted {
            Ok(pair) => pair,
            Err(error) => {
                stats.errors.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(%error,"accept failed; backing off");
                // Sleep is async: admitted sockets continue on runtime workers.
                tokio::select! {
                    _=shutdown.notified()=>break,
                    signal=&mut interrupt=>{result=signal;shutdown.request_shutdown();break;},
                    _=tokio::time::sleep(Duration::from_millis(50))=>{},
                }
                continue;
            }
        };
        let mut ticket = Ticket::new(Arc::clone(stats));
        let router = Arc::clone(router);
        let stats = Arc::clone(stats);
        let shutdown = shutdown.clone();
        // JoinSet::spawn uses tokio::spawn and tracks completion without one OS
        // thread per socket. Owned socket, permit and Arcs satisfy Send + 'static.
        tasks.spawn(
            async move {
                let _permit = permit;
                ticket.start();
                let result = handle_connection(stream, router, config, &stats, &shutdown).await;
                if let Err(error) = result {
                    stats.errors.fetch_add(1, Ordering::Relaxed);
                    tracing::debug!(%error,"connection failed");
                }
                tracing::debug!("connection finished");
                // Ticket/OwnedSemaphorePermit drop on every return and panic path.
            }
            .instrument(tracing::info_span!("connection",%peer)),
        );
    }
    drop(listener);
    tracing::info!(
        pending = tasks.len(),
        "shutdown: draining admitted connections"
    );
    // Never abort tasks or race write_all against shutdown. Await in-flight work.
    while let Some(joined) = tasks.join_next().await {
        report(Some(joined), stats);
    }
    tracing::info!("Tokio shutdown complete");
    result
}
fn report(joined: Option<Result<(), tokio::task::JoinError>>, stats: &Stats) {
    if let Some(Err(error)) = joined {
        if error.is_panic() {
            stats.panics.fetch_add(1, Ordering::Relaxed);
        } else {
            stats.errors.fetch_add(1, Ordering::Relaxed);
        }
        tracing::error!(%error,"connection task failed");
    }
}
async fn handle_connection(
    stream: TcpStream,
    router: Arc<Router>,
    config: Config,
    stats: &Stats,
    shutdown: &ShutdownHandle,
) -> Result<(), Error> {
    let mut connection = Connection::new(stream, config);
    for handled in 1..=config.max_requests {
        if shutdown.is_requested() && !connection.admit_ready()? {
            return Ok(());
        }
        let read = tokio::select! {
            result=connection.read_request()=>result,
            _=shutdown.notified()=>{
                if !connection.admit_ready()?{return Ok(());}
                connection.read_request().await
            },
        };
        let (mut response, head, keep_alive, version) = match read {
            Ok(Some(request)) => {
                stats.requests.fetch_add(1, Ordering::Relaxed);
                let head = request.method == Method::Head;
                let version = request.version;
                let persistent = request.keep_alive();
                let handler_router = Arc::clone(&router);
                let span = tracing::Span::current();
                let response = match tokio::task::spawn_blocking(move || {
                    span.in_scope(|| handler_router.handle(&request))
                })
                .await
                {
                    Ok(response) => response,
                    Err(error) => {
                        if error.is_panic() {
                            stats.panics.fetch_add(1, Ordering::Relaxed);
                        }
                        tracing::error!(%error,"blocking handler failed");
                        Response::text(Status::InternalServerError, "handler panicked")
                    }
                };
                let keep = persistent
                    && handled < config.max_requests
                    && (!shutdown.is_requested() || connection.has_pending())
                    && !matches!(
                        response.status,
                        Status::BadRequest
                            | Status::RequestTimeout
                            | Status::PayloadTooLarge
                            | Status::RequestHeaderFieldsTooLarge
                    );
                (response, head, keep, version)
            }
            Ok(None) | Err(Error::IdleTimeout) => return Ok(()),
            Err(Error::Parse(error)) => (
                Response::new(status_for(&error))
                    .header("Content-Type", "text/plain")
                    .body(error.to_string()),
                connection.pending_is_head(),
                false,
                Version::Http11,
            ),
            Err(Error::RequestTimeout) => (
                Response::text(Status::RequestTimeout, "request timeout"),
                connection.pending_is_head(),
                false,
                Version::Http11,
            ),
            Err(error) => return Err(error),
        };
        if !keep_alive {
            response = response.header("Connection", "close");
        } else if version == Version::Http10 {
            response = response.header("Connection", "keep-alive");
        }
        let written = connection.write_response(&response, head).await;
        if written.is_ok() {
            stats.responses.fetch_add(1, Ordering::Relaxed);
        }
        if !keep_alive {
            connection.linger_close().await;
            return written.map_err(Error::from);
        }
        written?;
    }
    Ok(())
}
