use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::{hint::black_box, time::Duration};
#[path = "../support/phase12/baseline_parser.rs"]
mod baseline;
use rust_serv::http::parser::parse_borrowed;
fn benchmarks(c: &mut Criterion) {
    let get = b"GET /health HTTP/1.1\r\nHost: x\r\nUser-Agent: benchmark\r\nAccept: */*\r\n\r\n";
    let mut fixed = b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Length: 4096\r\n\r\n".to_vec();
    fixed.resize(fixed.len() + 4096, 42);
    let mut chunked =
        b"POST /echo HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n1000\r\n".to_vec();
    chunked.resize(chunked.len() + 4096, 42);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    let mut group = c.benchmark_group("parser");
    group
        .sample_size(40)
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(3));
    for (name, wire) in [
        ("get", get.as_slice()),
        ("fixed4096", fixed.as_slice()),
        ("chunked4096", chunked.as_slice()),
    ] {
        group.throughput(Throughput::Bytes(wire.len() as u64));
        group.bench_function(format!("{name}_before"), |b| {
            b.iter(|| black_box(baseline::parse_request(black_box(wire)).unwrap().unwrap()))
        });
        group.bench_function(format!("{name}_borrowed"), |b| {
            b.iter(|| black_box(parse_borrowed(black_box(wire)).unwrap().unwrap()))
        });
    }
    group.throughput(Throughput::Bytes(get.len() as u64));
    group.bench_function("httparse_head_get", |b| {
        b.iter(|| {
            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut request = httparse::Request::new(&mut headers);
            black_box(request.parse(black_box(get)).unwrap());
            black_box(request.method);
            black_box(request.headers.len());
        })
    });
    group.finish();
}
criterion_group!(benches, benchmarks);
criterion_main!(benches);
