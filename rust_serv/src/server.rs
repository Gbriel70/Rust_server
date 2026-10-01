// Import necessary modules for I/O operations and networking
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};

// Import the parse_request function from the http::parser module
use crate::http::parser::parse_request;

// Define constants for the server address and buffer size
const ADDR: &str = "127.0.0.1:8080";
const BUFFER_SIZE: usize = 4096;

// Function to run the server, which listens for incoming TCP connections and handles them
pub fn run() -> io::Result<()> {
    // Create a TCP listener bound to the specified address
    let listener = TcpListener::bind(ADDR)?;

    println!("Listening on {}", listener.local_addr()?);

    // Loop to accept incoming connections and handle them
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if let Err(e) = handle_connection(stream) {
                    eprintln!("Connection error: {}", e);
                }
            }
            Err(e) => {
                eprintln!("accept error: {}", e);
            }
        }
    }

    Ok(())
}

// Function to handle individual connections
fn handle_connection(mut stream: TcpStream) -> io::Result<()> {
    // Create a buffer to store incoming data from the stream
    let mut buffer = Vec::with_capacity(BUFFER_SIZE);

    // Loop to read data from the stream and parse HTTP requests
    loop {
        // Create a temporary buffer to read data into
        let mut temp = [0u8; BUFFER_SIZE];

        // Read data from the stream into the temporary buffer
        let n = stream.read(&mut temp)?;

        // If no data was read, break the loop (connection closed)
        if n == 0 {
            break;
        }

        // Extend the main buffer with the newly read data
        buffer.extend_from_slice(&temp[..n]);

        match parse_request(&buffer) {
            Ok(None) => {
                // Request incomplete, continue reading more data
                continue;
            }

            // Request successfully parsed, handle the request and send a response
            Ok(Some((request, consumed))) => {
                println!("Request consumed: {} bytes", consumed);

                println!("Method: {:?}", request.method);
                println!("Target: {}", request.target);
                println!("Version: {:?}", request.version);

                // Print the headers of the request
                for (name, value) in request.headers.iter() {
                    println!("Header: {} = {}", name, value);
                }

                println!("Body: {:?}", request.body);

                // Create the response body
                let body = "hello\n";
    
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: text/plain\r\n\
                     Content-Length: {}\r\n\
                     Connection: close\r\n\
                     \r\n\
                     {}",
                    body.len(),
                    body
                );

                stream.write_all(response.as_bytes())?;

                break;
            }

            // Error occurred while parsing the HTTP request
            Err(e) => {
                eprintln!("HTTP parse error: {:?}", e);

                // Create the error response
                let response =
                    "HTTP/1.1 400 Bad Request\r\n\
                     Content-Length: 0\r\n\
                     Connection: close\r\n\
                     \r\n";

                stream.write_all(response.as_bytes())?;

                break;
            }
        }
    }

    Ok(())
}