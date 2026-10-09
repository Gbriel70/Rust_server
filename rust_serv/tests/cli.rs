use rust_serv::{cli::Options, server::Execution};
fn parse(args: &[&str]) -> std::io::Result<Options> {
    Options::parse(args.iter().map(|s| (*s).to_owned()))
}
#[test]
fn defaults() {
    let options = parse(&[]).unwrap();
    assert_eq!(options.root, "./public");
    assert_eq!(
        options.execution,
        Execution::Pool {
            workers: 4,
            queue_capacity: 64
        }
    );
}
#[test]
fn modes_and_options() {
    let options = parse(&[
        "--mode",
        "thread",
        "--root",
        "/tmp/site",
        "--addr",
        "127.0.0.1:0",
    ])
    .unwrap();
    assert_eq!(options.execution, Execution::ThreadPerConnection);
    assert_eq!(options.root, "/tmp/site");
    let options = parse(&["--workers", "8", "--queue", "128"]).unwrap();
    assert_eq!(
        options.execution,
        Execution::Pool {
            workers: 8,
            queue_capacity: 128
        }
    );
}
#[test]
fn rejects_invalid_values() {
    for args in [
        &["--mode", "bogus"][..],
        &["--workers", "0"],
        &["--workers", "-1"],
        &["--queue", "0"],
        &["--root"],
        &["--unexpected", "x"],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}
