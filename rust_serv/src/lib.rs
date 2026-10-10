//! HTTP server library and its reusable components.

pub mod connection;
pub mod error;
pub mod http;
pub mod router;
pub mod routes;

pub mod server;

pub mod static_files;

pub mod cli;
pub mod stats;
pub mod thread_pool;

#[cfg(target_os = "linux")]
pub mod epoll;
#[cfg(target_os = "linux")]
pub mod event_loop;

pub mod tokio_io;
