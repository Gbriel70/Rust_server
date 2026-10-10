//! Concurrent TCP server: immutable Arc<Router>, connection jobs and graceful drain.
use crate::{
    connection::{Config, Connection},
    error::Error,
    http::{
        parser::ParseError,
        request::{Method, Version},
        response::{Response, Status},
    },
    router::Router,
    stats::{Stats, Ticket},
    thread_pool::ThreadPool,
};
use std::{
    io,
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Execution {
    ThreadPerConnection,
    Epoll {
        edge_triggered: bool,
    },
    Pool {
        workers: usize,
        queue_capacity: usize,
    },
}
impl Execution {
    pub fn validate(self) -> io::Result<()> {
        if matches!(
            self,
            Self::Pool { workers: 0, .. }
                | Self::Pool {
                    queue_capacity: 0,
                    ..
                }
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "workers and queue capacity must be positive",
            ));
        }
        #[cfg(not(target_os = "linux"))]
        if matches!(self, Self::Epoll { .. }) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "epoll requires Linux",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct ShutdownHandle(Arc<AtomicBool>);
impl ShutdownHandle {
    pub fn request_shutdown(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct Server {
    listener: TcpListener,
    router: Arc<Router>,
    config: Config,
    execution: Execution,
    shutdown: ShutdownHandle,
    running: AtomicBool,
    stats: Arc<Stats>,
}
impl Server {
    pub fn bind(addr: impl ToSocketAddrs, router: Router) -> io::Result<Self> {
        Self::bind_with_config(addr, router, Config::default())
    }
    pub fn bind_with_config(
        addr: impl ToSocketAddrs,
        router: Router,
        config: Config,
    ) -> io::Result<Self> {
        config.validate()?;
        let stats = Arc::new(Stats::default());
        let handler_stats = Arc::clone(&stats);
        // /stats is reserved; an exact application route shadows a file of this name.
        let router = router.route(Method::Get, "/stats", move |_| handler_stats.response());
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            router: Arc::new(router),
            config,
            execution: Execution::ThreadPerConnection,
            shutdown: ShutdownHandle::default(),
            running: AtomicBool::new(false),
            stats,
        })
    }
    pub fn with_execution(mut self, execution: Execution) -> io::Result<Self> {
        execution.validate()?;
        self.execution = execution;
        Ok(self)
    }
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        self.shutdown.clone()
    }
    pub fn stats(&self) -> Arc<Stats> {
        Arc::clone(&self.stats)
    }

    /// Stop accepting on shutdown, then drain all admitted jobs and join threads.
    /// An active request is not interrupted. Idle sockets remain subject to Config;
    /// arbitrary handlers and local filesystem I/O have no total shutdown deadline.
    pub fn run(&self) -> io::Result<()> {
        if self.running.swap(true, Ordering::AcqRel) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "server can only run once",
            ));
        }
        if let Execution::Epoll { edge_triggered } = self.execution {
            #[cfg(target_os = "linux")]
            return crate::event_loop::run(
                &self.listener,
                &self.router,
                self.config,
                &self.stats,
                &self.shutdown,
                edge_triggered,
            );
            #[cfg(not(target_os = "linux"))]
            {
                let _ = edge_triggered;
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "epoll requires Linux",
                ));
            }
        }
        let mut pool = match self.execution {
            Execution::ThreadPerConnection | Execution::Epoll { .. } => None,
            Execution::Pool {
                workers,
                queue_capacity,
            } => Some(ThreadPool::with_capacity(workers, queue_capacity)?),
        };
        let mut threads: Vec<JoinHandle<()>> = Vec::new();
        while !self.shutdown.is_requested() {
            // Finished thread handles must not accumulate across the server's lifetime.
            let mut i = 0;
            while i < threads.len() {
                if threads[i].is_finished() {
                    let _ = threads.swap_remove(i).join();
                } else {
                    i += 1;
                }
            }
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    self.stats.errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!("accept error (backing off): {error}");
                    // Includes EMFILE/ENFILE: do not spin or repeatedly exhaust CPU.
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            let mut ticket = Ticket::new(Arc::clone(&self.stats));
            // Explicitly enforce blocking streams even on platforms that inherit flags.
            if let Err(error) = stream.set_nonblocking(false) {
                self.stats.errors.fetch_add(1, Ordering::Relaxed);
                eprintln!("stream setup error: {error}");
                continue;
            }
            let router = Arc::clone(&self.router);
            let stats = Arc::clone(&self.stats);
            let shutdown = self.shutdown.clone();
            let config = self.config;
            let job = move || {
                ticket.start();
                let result = catch_unwind(AssertUnwindSafe(|| {
                    handle_connection(stream, &router, config, &stats, &shutdown)
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        stats.errors.fetch_add(1, Ordering::Relaxed);
                        eprintln!("connection error: {error}");
                    }
                    Err(_) => {
                        stats.panics.fetch_add(1, Ordering::Relaxed);
                        eprintln!("connection handler panicked; socket closed");
                    }
                }
                // ticket Drop restores active even after a caught handler panic.
            };
            let result = if let Some(pool) = &pool {
                pool.execute(job)
            } else {
                thread::Builder::new()
                    .name("http-connection".into())
                    .spawn(job)
                    .map(|handle| threads.push(handle))
            };
            if let Err(error) = result {
                eprintln!("connection rejected: {error}");
            }
        }
        // Never join a worker while a live sender keeps its recv blocked.
        let pool_result = if let Some(pool) = &mut pool {
            pool.shutdown()
        } else {
            Ok(())
        };
        let mut failed = false;
        for handle in threads {
            failed |= handle.join().is_err();
        }
        pool_result?;
        if failed {
            Err(io::Error::other(
                "connection thread panicked outside handler",
            ))
        } else {
            Ok(())
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    router: &Router,
    config: Config,
    stats: &Stats,
    shutdown: &ShutdownHandle,
) -> Result<(), Error> {
    let mut connection = Connection::new(stream, config)?;
    for handled in 1..=config.max_requests {
        if handled > 1 && shutdown.is_requested() && !connection.has_pending_request() {
            connection.linger_close();
            return Ok(());
        }
        let (mut response, head_only, keep_alive, version) = match connection.read_request() {
            Ok(Some(request)) => {
                stats.requests.fetch_add(1, Ordering::Relaxed);
                let response = router.handle(&request);
                let keep_alive = request.keep_alive()
                    && (!shutdown.is_requested() || connection.has_pending_request())
                    && handled < config.max_requests
                    && !matches!(
                        response.status,
                        Status::BadRequest
                            | Status::RequestTimeout
                            | Status::PayloadTooLarge
                            | Status::RequestHeaderFieldsTooLarge
                    );
                (
                    response,
                    request.method == Method::Head,
                    keep_alive,
                    request.version,
                )
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
        let result = connection.write_response(&response, head_only);
        if result.is_ok() {
            stats.responses.fetch_add(1, Ordering::Relaxed);
        }
        if !keep_alive {
            connection.linger_close();
            return result.map_err(Error::from);
        }
        result?;
    }
    Ok(())
}

/// Maps parsing errors to appropriate HTTP statuses for the client.
pub fn status_for(error: &ParseError) -> Status {
    match error {
        ParseError::MalformedRequestLine
        | ParseError::MalformedHeader
        | ParseError::MissingHost
        | ParseError::InvalidContentLength => Status::BadRequest,
        ParseError::BodyTooLarge => Status::PayloadTooLarge,
        ParseError::HeadersTooLarge => Status::RequestHeaderFieldsTooLarge,
        ParseError::UnsupportedMethod | ParseError::UnsupportedTransferEncoding => {
            Status::NotImplemented
        }
        ParseError::UnsupportedVersion => Status::HttpVersionNotSupported,
    }
}
