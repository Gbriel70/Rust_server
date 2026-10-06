use rust_serv::http::date::{format_http_date, parse_http_date};
use std::time::{Duration, UNIX_EPOCH};
#[test]
fn vectors_and_roundtrips() {
    assert_eq!(
        format_http_date(UNIX_EPOCH),
        "Thu, 01 Jan 1970 00:00:00 GMT"
    );
    assert_eq!(
        format_http_date(UNIX_EPOCH + Duration::from_secs(784111777)),
        "Sun, 06 Nov 1994 08:49:37 GMT"
    );
    for s in [
        0, 1, 86399, 86400, 784111777, 951782400, 1709164800, 1735689599, 1735689600, 4107542400,
    ] {
        let time = UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(parse_http_date(&format_http_date(time)), Some(time));
    }
    assert_eq!(
        format_http_date(UNIX_EPOCH + Duration::from_secs(951782400)),
        "Tue, 29 Feb 2000 00:00:00 GMT"
    );
    assert_eq!(
        format_http_date(UNIX_EPOCH + Duration::from_secs(1709164800)),
        "Thu, 29 Feb 2024 00:00:00 GMT"
    );
}
#[test]
fn invalid() {
    for s in [
        "",
        "garbage",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        "Sun, 31 Feb 2024 00:00:00 GMT",
        "Thu, 01 Jan 1970 24:00:00 GMT",
        "Thu, 01 Jan 1970 00:00:60 GMT",
        "Fri, 01 Jan 1970 00:00:00 GMT",
    ] {
        assert_eq!(parse_http_date(s), None);
    }
}
