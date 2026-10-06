use rust_serv::http::uri::{UriError, percent_decode};

#[test]
fn percent_decoding_table() {
    for (input, expected) in [
        ("", Ok("")),
        ("hello", Ok("hello")),
        ("café", Ok("café")),
        ("%20", Ok(" ")),
        ("caf%C3%A9", Ok("café")),
        ("%F0%9F%99%82", Ok("🙂")),
        ("%2f", Ok("/")),
        ("%2F", Ok("/")),
        ("a+b", Ok("a+b")),
        ("%25", Ok("%")),
        ("%252F", Ok("%2F")),
        ("%3F", Ok("?")),
        ("%00", Ok("\0")),
        ("%5c", Ok("\\")),
        ("%", Err(UriError::InvalidEscape)),
        ("%4", Err(UriError::InvalidEscape)),
        ("%zz", Err(UriError::InvalidEscape)),
        ("%0x", Err(UriError::InvalidEscape)),
        ("%é", Err(UriError::InvalidEscape)),
        ("%FF", Err(UriError::InvalidUtf8)),
        ("%C3", Err(UriError::InvalidUtf8)),
        ("%C0%AF", Err(UriError::InvalidUtf8)),
    ] {
        assert_eq!(
            percent_decode(input).as_deref(),
            expected.as_ref().map(|s| *s),
            "{input}"
        );
    }
}
