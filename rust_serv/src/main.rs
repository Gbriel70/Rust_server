// Import the server module from the rust_serv crate
use rust_serv::server;

// The main function serves as the entry point of the application
fn main() -> std::io::Result<()> {
    server::run()?;
    Ok(())
}
