//! Configuration parsing kept separate from server business logic.
use crate::server::Execution;
use std::io;

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub root: String,
    pub addr: String,
    pub execution: Execution,
}
impl Options {
    pub fn parse(args: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut args = args.into_iter();
        let mut root = "./public".to_owned();
        let mut addr = "127.0.0.1:8080".to_owned();
        let mut mode = "pool".to_owned();
        let mut workers = 4;
        let mut queue_capacity = 64;
        while let Some(arg) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| invalid("option requires a value"))?;
            match arg.as_str() {
                "--root" => root = value,
                "--addr" => addr = value,
                "--mode" => mode = value,
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
                        "usage: rust_serv [--root dir] [--addr host:port] [--mode thread|pool] [--workers N] [--queue N]",
                    ));
                }
            }
        }
        if workers == 0 || queue_capacity == 0 {
            return Err(invalid("workers and queue must be positive"));
        }
        let execution = match mode.as_str() {
            "thread" => Execution::ThreadPerConnection,
            "pool" => Execution::Pool {
                workers,
                queue_capacity,
            },
            _ => return Err(invalid("--mode must be thread or pool")),
        };
        Ok(Self {
            root,
            addr,
            execution,
        })
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
