use rust_serv::http::request::{Headers, Method, Request, Version};
use rust_serv::http::response::{Response, Status};
use rust_serv::router::Router;
use rust_serv::routes::default_router;

fn req(method: Method, target: &str) -> Request {
    Request {
        method,
        target: target.into(),
        version: Version::Http11,
        headers: Headers::new(),
        body: Vec::new(),
    }
}

#[test]
fn unknown_path_is_404() {
    assert_eq!(
        default_router().handle(&req(Method::Get, "/nope")).status,
        Status::NotFound
    );
}

#[test]
fn known_path_with_wrong_method_is_405() {
    let response = default_router().handle(&req(Method::Delete, "/health"));
    assert_eq!(response.status, Status::MethodNotAllowed);
    assert_eq!(response.headers.get("Allow"), Some("GET, HEAD"));
}

#[test]
fn allow_is_sorted_and_duplicate_registration_replaces_handler() {
    let router = Router::new()
        .route(Method::Put, "/x", |_| Response::new(Status::Ok))
        .route(Method::Get, "/x", |_| Response::text(Status::Ok, "old"))
        .route(Method::Delete, "/x", |_| Response::new(Status::Ok))
        .route(Method::Post, "/x", |_| Response::new(Status::Ok))
        .route(Method::Get, "/x", |_| Response::text(Status::Ok, "new"));
    let response = router.handle(&req(Method::Options, "/x"));
    assert_eq!(
        response.headers.get("Allow"),
        Some("DELETE, GET, HEAD, POST, PUT")
    );
    assert_eq!(router.handle(&req(Method::Get, "/x")).body, b"new");
}

#[test]
fn head_calls_the_get_handler_and_retains_the_representation() {
    let response = default_router().handle(&req(Method::Head, "/health"));
    assert_eq!(response.status, Status::Ok);
    assert_eq!(response.body, b"ok\n");
}

#[test]
fn head_without_get_is_405() {
    let response = default_router().handle(&req(Method::Head, "/echo"));
    assert_eq!(response.status, Status::MethodNotAllowed);
    assert_eq!(response.headers.get("Allow"), Some("POST"));
}

#[test]
fn trailing_slash_is_significant() {
    assert_eq!(
        default_router()
            .handle(&req(Method::Get, "/health/"))
            .status,
        Status::NotFound
    );
}

#[test]
fn query_is_ignored_when_matching_the_path() {
    assert_eq!(
        default_router()
            .handle(&req(Method::Get, "/health?x=1"))
            .status,
        Status::Ok
    );
}

#[test]
fn repeated_slashes_are_significant() {
    assert_eq!(
        default_router()
            .handle(&req(Method::Get, "//health"))
            .status,
        Status::NotFound
    );
}

#[test]
fn paths_are_case_sensitive() {
    assert_eq!(
        default_router().handle(&req(Method::Get, "/Health")).status,
        Status::NotFound
    );
}

#[test]
fn dot_segments_are_not_normalized() {
    for path in ["/a/../health", "/a/%2e%2e/health"] {
        assert_eq!(
            default_router().handle(&req(Method::Get, path)).status,
            Status::NotFound
        );
    }
}

#[test]
fn unicode_path_is_decoded_before_lookup() {
    let router = Router::new().route(Method::Get, "/café", |_| {
        Response::text(Status::Ok, "coffee")
    });
    assert_eq!(
        router.handle(&req(Method::Get, "/caf%C3%A9")).body,
        b"coffee"
    );
}

#[test]
fn invalid_escapes_are_400_even_on_unknown_paths() {
    for path in ["/%zz", "/%4", "/%", "/%é"] {
        assert_eq!(
            default_router().handle(&req(Method::Get, path)).status,
            Status::BadRequest
        );
    }
}

#[test]
fn encoded_separators_are_400() {
    for path in ["/a%2Fb", "/a%2fb", "/a%5Cb", "/a%5cb"] {
        assert_eq!(
            default_router().handle(&req(Method::Get, path)).status,
            Status::BadRequest
        );
    }
}

#[test]
fn encoded_nul_is_400() {
    assert_eq!(
        default_router().handle(&req(Method::Get, "/x%00")).status,
        Status::BadRequest
    );
}

#[test]
fn invalid_decoded_utf8_is_400() {
    assert_eq!(
        default_router().handle(&req(Method::Get, "/%FF")).status,
        Status::BadRequest
    );
}

#[test]
fn encoded_question_mark_stays_in_the_path_and_decoding_occurs_once() {
    let router = Router::new()
        .route(Method::Get, "/a?b", |_| {
            Response::text(Status::Ok, "question")
        })
        .route(Method::Get, "/%2F", |_| {
            Response::text(Status::Ok, "percent")
        });
    assert_eq!(
        router.handle(&req(Method::Get, "/a%3Fb?x=1")).body,
        b"question"
    );
    assert_eq!(router.handle(&req(Method::Get, "/%252F")).body, b"percent");
}

#[test]
fn captured_state_survives_calls_and_head_also_increments() {
    let router = default_router();
    assert_eq!(router.handle(&req(Method::Get, "/counter")).body, b"1");
    assert_eq!(router.handle(&req(Method::Head, "/counter")).body, b"2");
    assert_eq!(router.handle(&req(Method::Get, "/counter")).body, b"3");
    assert_eq!(
        default_router().handle(&req(Method::Get, "/counter")).body,
        b"1"
    );
}

#[test]
fn hello_decodes_query_components_without_form_encoding() {
    let router = default_router();
    for (target, expected) in [
        ("/hello", "hello, world"),
        ("/hello?", "hello, world"),
        ("/hello?name=", "hello, "),
        ("/hello?name", "hello, "),
        ("/hello?name=a+b", "hello, a+b"),
        ("/hello?name=a%20b", "hello, a b"),
        ("/hello?name=%F0%9F%99%82", "hello, 🙂"),
        ("/hello?x=1&na%6De=a%26b%3Dc", "hello, a&b=c"),
        ("/hello?name=first&name=second", "hello, first"),
        ("/hello?name=a%3Fb", "hello, a?b"),
    ] {
        let response = router.handle(&req(Method::Get, target));
        assert_eq!(response.status, Status::Ok, "{target}");
        assert_eq!(response.body, expected.as_bytes(), "{target}");
    }
    for target in [
        "/hello?name=%zz",
        "/hello?name=%FF",
        "/hello?%zz=x",
        "/hello?name=ok&name=%zz",
    ] {
        assert_eq!(
            router.handle(&req(Method::Get, target)).status,
            Status::BadRequest,
            "{target}"
        );
    }
}

#[test]
#[should_panic(expected = "register GET to provide HEAD automatically")]
fn explicit_head_registration_is_rejected() {
    Router::new().route(Method::Head, "/x", |_| Response::new(Status::Ok));
}
