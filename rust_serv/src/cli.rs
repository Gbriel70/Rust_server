//! Configuration parsing kept separate from server business logic.
use crate::server::Execution;
use std::{io, time::Duration};

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub root: String,
    pub addr: String,
    pub idle_timeout: Duration,
    pub execution: Execution,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut args = args.into_iter();
        let mut root = "./public".to_owned();
        let mut addr = "127.0.0.1:8080".to_owned();
        let mut idle_seconds = 5u64;
        let mut mode = "pool".to_owned();
        let mut trigger = "level".to_owned();
        let mut max_connections = 16_384;
        let mut workers = 4;
        let mut queue_capacity = 64;
        while let Some(arg) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| invalid("option requires a value"))?;
            match arg.as_str() {
                "--root" => root = value,
                "--addr" => addr = value,
                "--idle-timeout" => {
                    idle_seconds = value.parse().map_err(|_| invalid("invalid idle timeout"))?
                }
                "--mode" => mode = value,
                "--trigger" => trigger = value,
                "--max-connections" => {
                    max_connections = value
                        .parse()
                        .map_err(|_| invalid("invalid connection limit"))?
                }
                "--workers" => {
                    workers = value.parse().map_err(|_| invalid("invalid worker count"))?
                }
                "--queue" => {
                    queue_capacity = value
                        .parse()
                        .map_err(|_| invalid("invalid queue capacity"))?
                }
                _ => {
                    return Err(invalid(
                        "usage: rust_serv [--root dir] [--addr host:port] [--idle-timeout seconds] [--mode thread|pool|epoll|tokio] [--max-connections N] [--trigger level|edge] [--workers N] [--queue N]",
                    ));
                }
            }
        }
        if idle_seconds == 0
            || std::time::Instant::now()
                .checked_add(Duration::from_secs(idle_seconds))
                .is_none()
        {
            return Err(invalid("idle timeout must be positive and representable"));
        }
        if workers == 0 || queue_capacity == 0 {
            return Err(invalid("workers and queue must be positive"));
        }
        let edge_triggered = match trigger.as_str() {
            "level" => false,
            "edge" => true,
            _ => return Err(invalid("--trigger must be level or edge")),
        };
        let execution = match mode.as_str() {
            "thread" => Execution::ThreadPerConnection,
            "pool" => Execution::Pool {
                workers,
                queue_capacity,
            },
            "epoll" => Execution::Epoll { edge_triggered },
            "tokio" => Execution::Tokio { max_connections },
            _ => return Err(invalid("--mode must be thread, pool, epoll or tokio")),
        };
        execution.validate()?;
        Ok(Self {
            root,
            addr,
            idle_timeout: Duration::from_secs(idle_seconds),
            execution,
        })
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
