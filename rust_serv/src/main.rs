use rust_serv::server::Server;

/// Starts the TCP server and keeps it accepting connections on port 8080.
fn main() -> std::io::Result<()> {
    let server = Server::bind("127.0.0.1:8080")?;
    println!("Listening on {}", server.local_addr()?);
    server.run()
}
