//! Server TCP loop and request-to-response adaptation.

use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};

use crate::error::Error;
use crate::http::parser::ParseError;
use crate::http::request::{Method, Version};
use crate::http::response::{Response, Status};
use crate::router::Router;

use crate::connection::{Config, Connection};

/// TCP server responsible for receiving HTTP requests and sending responses.
pub struct Server {
    listener: TcpListener,
    router: Router,
    config: Config,
}

impl Server {
    /// Opens a TCP listener at the provided address.
    pub fn bind(addr: impl ToSocketAddrs, router: Router) -> io::Result<Self> {
        Self::bind_with_config(addr, router, Config::default())
    }

    pub fn bind_with_config(
        addr: impl ToSocketAddrs,
        router: Router,
        config: Config,
    ) -> io::Result<Self> {
        config.validate()?;
        Ok(Self {
            listener: TcpListener::bind(addr)?,
            router,
            config,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Continuously accepts connections and handles them sequentially.
    pub fn run(&self) -> io::Result<()> {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    if let Err(error) = handle_connection(stream, &self.router, self.config) {
                        eprintln!("Connection error: {error}");
                    }
                }
                Err(error) => eprintln!("accept error: {error}"),
            }
        }
        Ok(())
    }
}

fn handle_connection(stream: TcpStream, router: &Router, config: Config) -> Result<(), Error> {
    let mut connection = Connection::new(stream, config)?;
    for handled in 1..=config.max_requests {
        let (mut response, head_only, keep_alive, version) = match connection.read_request() {
            Ok(Some(request)) => {
                let response = router.handle(&request);
                let keep_alive = request.keep_alive()
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
