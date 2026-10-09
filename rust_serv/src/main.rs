use rust_serv::{cli::Options, routes, server::Server, static_files::StaticFiles};

fn main() -> std::io::Result<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    let server = Server::bind(
        &options.addr,
        routes::static_router(StaticFiles::new(options.root)?),
    )?
    .with_execution(options.execution)?;
    let shutdown = server.shutdown_handle();
    // std has no portable Ctrl-C handler. Explicit Enter is the graceful CLI trigger;
    // EOF is ignored so a daemon with closed stdin does not stop immediately.
    std::thread::Builder::new()
        .name("shutdown-stdin".into())
        .spawn(move || {
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
                shutdown.request_shutdown();
            }
        })?;
    println!(
        "Listening on {} ({:?}); press Enter for graceful shutdown",
        server.local_addr()?,
        options.execution
    );
    server.run()
}
