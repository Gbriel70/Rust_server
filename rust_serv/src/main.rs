use rust_serv::{routes, server::Server, static_files::StaticFiles};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut root = String::from("./public");
    
    while let Some(arg) = args.next() {
        if arg != "--root" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "usage: rust_serv [--root <dir>]",
            ));
        }
        root = args.next().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--root requires a directory",
            )
        })?;
    }
    Server::bind(
        "127.0.0.1:8080",
        routes::static_router(StaticFiles::new(root)?),
    )?
    .run()
}
