use rust_serv::{
    cli::Options,
    connection::Config,
    routes,
    server::{Execution, Server},
    static_files::StaticFiles,
};

#[tokio::main(worker_threads = 4)]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rust_serv=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let options = Options::parse(std::env::args().skip(1))?;
    let server = Server::bind_with_config(
        &options.addr,
        routes::static_router(StaticFiles::new(options.root)?),
        Config {
            idle_timeout: options.idle_timeout,
            ..Config::default()
        },
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
        "Listening on {} ({:?}); press Enter for graceful shutdown (Tokio also accepts Ctrl-C)",
        server.local_addr()?,
        options.execution
    );
    if matches!(options.execution, Execution::Tokio { .. }) {
        server.run_async().await
    } else {
        server.run()
    }
}
