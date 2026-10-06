use rust_serv::http::headers::Headers;
use rust_serv::http::request::{Method, Request, Version};

#[test]
fn connection_tokens_across_lines_are_case_insensitive_and_exact() {
    let mut headers = Headers::new();
    headers.push("cOnNeCtIoN".into(), "\tUpgrade , Keep-Alive\t, ,".into());
    headers.push("Connection".into(), " CLOSE ".into());
    headers.push("X-Other".into(), "hidden".into());
    for (token, expected) in [
        ("keep-alive", true),
        ("UPGRADE", true),
        ("close", true),
        ("alive", false),
        ("closed", false),
        ("hidden", false),
        ("", false),
    ] {
        assert_eq!(headers.has_token("CONNECTION", token), expected, "{token}");
    }
    assert!(!headers.has_token("Missing", "close"));
}

#[test]
fn keep_alive_version_and_tokens_table() {
    for (version, values, expected) in [
        (Version::Http11, vec![], true),
        (Version::Http11, vec!["close"], false),
        (Version::Http11, vec!["keep-alive"], true),
        (Version::Http11, vec!["CLOSE"], false),
        (Version::Http11, vec!["keep-alive, close"], false),
        (Version::Http11, vec!["Keep-Alive, Upgrade"], true),
        (Version::Http11, vec!["keep-alive", "close"], false),
        (Version::Http11, vec!["close", "keep-alive"], false),
        (Version::Http11, vec!["disclose"], true),
        (Version::Http10, vec![], false),
        (Version::Http10, vec!["close"], false),
        (Version::Http10, vec!["keep-alive"], true),
        (Version::Http10, vec!["Keep-Alive, Upgrade"], true),
        (Version::Http10, vec!["keep-alive, close"], false),
        (Version::Http10, vec!["keep-alive", "CLOSE"], false),
        (Version::Http10, vec!["upgrade", "\tKEEP-ALIVE "], true),
        (Version::Http10, vec!["x-keep-alive"], false),
    ] {
        let mut headers = Headers::new();
        for value in &values {
            headers.push("Connection".into(), (*value).into());
        }
        let request = Request {
            method: Method::Get,
            target: "/".into(),
            version,
            headers,
            body: Vec::new(),
        };
        assert_eq!(request.keep_alive(), expected, "{version:?} {values:?}");
    }
}
