//! Server TCP loop and request-to-response adaptation.

use std::io::{self, Read};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};

use crate::error::Error;
use crate::http::parser::{ParseError, parse_request};
use crate::http::request::Request;
use crate::http::response::{Response, Status};

const BUFFER_SIZE: usize = 1024;

/// TCP server responsible for receiving HTTP requests and sending responses.
pub struct Server {
    listener: TcpListener,
}

impl Server {
    /// Opens a TCP listener at the provided address.
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        Ok(Self {
            listener: TcpListener::bind(addr)?,
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
                    if let Err(error) = handle_connection(stream) {
                        eprintln!("Connection error: {error}");
                    }
                }
                Err(error) => eprintln!("accept error: {error}"),
            }
        }
        Ok(())
    }
}

fn read_request(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
) -> Result<Option<(Request, usize)>, Error> {
    // The parser may need multiple reads before the complete request is available.
    loop {
        if let Some(request) = parse_request(buf)? {
            return Ok(Some(request));
        }
        let mut chunk = [0; BUFFER_SIZE];
        let n = match stream.read(&mut chunk) {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if n == 0 {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Err(Error::UnexpectedEof)
            };
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn handle_connection(mut stream: TcpStream) -> Result<(), Error> {
    let mut buf = Vec::with_capacity(BUFFER_SIZE);
    // Each connection produces at most one response and is then closed.
    let response = match read_request(&mut stream, &mut buf) {
        Ok(Some((request, _))) => handle(&request),
        Ok(None) => return Ok(()),
        Err(Error::Parse(error)) => Response::new(status_for(&error))
            .header("Content-Type", "text/plain")
            .body(error.to_string()),
        Err(error) => return Err(error),
    };
    response
        .header("Connection", "close")
        .write_to(&mut stream)?;
    Ok(())
}

fn handle(_request: &Request) -> Response {
    Response::new(Status::Ok)
        .header("Content-Type", "text/plain")
        .body("hello")
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
