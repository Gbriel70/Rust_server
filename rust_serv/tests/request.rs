use rust_serv::http::request::{Headers, Method, Request, Version};

#[test]
fn path_and_query_borrow_the_original_target() {
    for (target, path, query) in [
        ("/a?b=c", "/a", Some("b=c")),
        ("/a?", "/a", Some("")),
        ("/a?b?c", "/a", Some("b?c")),
        ("/a", "/a", None),
        ("/a%3Fb?x=%2F", "/a%3Fb", Some("x=%2F")),
    ] {
        let request = Request {
            method: Method::Get,
            target: target.into(),
            version: Version::Http11,
            headers: Headers::new(),
            body: Vec::new(),
        };
        assert_eq!(request.path(), path);
        assert_eq!(request.query(), query);
        assert_eq!(request.path().as_ptr(), request.target.as_ptr());
    }
}

#[test]
fn methods_round_trip_through_their_names() {
    for (method, name) in [
        (Method::Get, "GET"),
        (Method::Head, "HEAD"),
        (Method::Post, "POST"),
        (Method::Put, "PUT"),
        (Method::Delete, "DELETE"),
        (Method::Options, "OPTIONS"),
    ] {
        assert_eq!(method.as_str(), name);
        assert_eq!(method.to_string(), name);
        assert_eq!(name.parse::<Method>(), Ok(method));
    }
}
