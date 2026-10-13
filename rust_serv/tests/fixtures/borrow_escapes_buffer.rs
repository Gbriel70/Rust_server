struct Request<'a> { target: &'a str }
fn request<'a>() -> Request<'a> {
    let buf = String::from("/health");
    Request { target: &buf }
}
fn main() { let _ = request(); }
