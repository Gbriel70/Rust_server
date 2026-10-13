struct Request<'a> { target: &'a str }
struct Connection<'a> { buf: String, request: Request<'a> }
fn build<'a>() -> Connection<'a> {
    let buf = String::from("/health");
    let request = Request { target: &buf };
    Connection { buf, request }
}
fn main() { let _ = build(); }
