use proptest::prelude::*;
use rust_serv::http::{
    borrowed_request::Request,
    parser::{RequestParser, parse_borrowed, parse_request},
    request::Method,
};
use std::borrow::Cow;
#[path = "support/phase12/baseline_parser.rs"]
mod baseline;
fn serialize(request: &Request<'_>) -> Vec<u8> {
    let version = match request.version {
        rust_serv::http::request::Version::Http10 => "HTTP/1.0",
        rust_serv::http::request::Version::Http11 => "HTTP/1.1",
    };
    let mut wire = format!("{} {} {version}\r\n", request.method, request.target).into_bytes();
    for (name, value) in request.headers.iter() {
        wire.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
    }
    wire.extend_from_slice(b"\r\n");
    if request.headers.has_token("Transfer-Encoding", "chunked") {
        for chunk in request.body.chunks(17) {
            wire.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            wire.extend_from_slice(chunk);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b"0\r\n\r\n");
    } else {
        wire.extend_from_slice(&request.body);
    }
    wire
}
proptest! {
 #[test]
 fn fixed_request_round_trip(path in "/[a-zA-Z0-9/_%-]{0,64}",body in prop::collection::vec(any::<u8>(),0..1024),value in "[a-zA-Z0-9 ]{0,50}") {
  let mut wire=format!("POST {path} HTTP/1.1\r\nHost: roundtrip\r\nX-Test: {value}\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();wire.extend_from_slice(&body);
  let (first,n)=parse_borrowed(&wire).unwrap().unwrap();prop_assert_eq!(n,wire.len());
  let encoded=serialize(&first);let (second,n)=parse_borrowed(&encoded).unwrap().unwrap();prop_assert_eq!(n,encoded.len());prop_assert_eq!(first,second);
 }
 #[test]
 fn arbitrary_bytes_match_phase11_and_never_overconsume(wire in prop::collection::vec(any::<u8>(),0..16384)) {
  let before=baseline::parse_request(&wire).map_err(|e|e.to_string());let after=parse_request(&wire).map_err(|e|e.to_string());prop_assert_eq!(before,after);
  if let Ok(Some((_,consumed)))=parse_borrowed(&wire){prop_assert!(consumed>0 && consumed<=wire.len());}
 }
 #[test]
 fn fragmented_chunked_round_trip(body in prop::collection::vec(any::<u8>(),0..1024),step in 1usize..64) {
  let mut wire=b"POST /echo HTTP/1.1\r\nHost: roundtrip\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
  for chunk in body.chunks(17){wire.extend_from_slice(format!("{:x}\r\n",chunk.len()).as_bytes());wire.extend_from_slice(chunk);wire.extend_from_slice(b"\r\n");}wire.extend_from_slice(b"0\r\n\r\n");
  let (first,_)=parse_borrowed(&wire).unwrap().unwrap();let encoded=serialize(&first);
  let (second,n)=parse_borrowed(&encoded).unwrap().unwrap();prop_assert_eq!(n,encoded.len());prop_assert_eq!(first,second);
  let expected=baseline::parse_request(&wire).unwrap().unwrap();let mut parser=RequestParser::default();
  let mut found=None;for end in (step..wire.len()).step_by(step).chain(std::iter::once(wire.len())){if let Some(result)=parser.parse_borrowed(&wire[..end]).unwrap(){found=Some((result.0.into_owned_request(),result.1));break;}}
  prop_assert_eq!(found,Some(expected));
 }
}
#[test]
fn target_headers_and_fixed_body_point_into_input() {
    let wire =
        b"POST /echo?q=1 HTTP/1.1\r\nHost: x\r\nX-Test: value\r\nContent-Length: 3\r\n\r\na\0b";
    let (request, n) = parse_borrowed(wire).unwrap().unwrap();
    assert_eq!(n, wire.len());
    assert!(matches!(request.target, Cow::Borrowed(_)));
    assert!(matches!(request.body, Cow::Borrowed(_)));
    let range = wire.as_ptr() as usize..wire.as_ptr() as usize + wire.len();
    assert!(range.contains(&(request.target.as_ptr() as usize)));
    assert!(range.contains(&(request.body.as_ptr() as usize)));
    for (name, value) in request.headers.iter() {
        assert!(range.contains(&(name.as_ptr() as usize)));
        assert!(range.contains(&(value.as_ptr() as usize)));
    }
}
#[test]
fn chunked_is_owned_only_when_contiguous_data_requires_it() {
    let wire=b"POST /echo HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n1\r\na\r\n1\r\nb\r\n0\r\n\r\n";
    let (request, _) = parse_borrowed(wire).unwrap().unwrap();
    assert!(matches!(request.body, Cow::Owned(_)));
    assert_eq!(request.body.as_ref(), b"ab");
    assert!(matches!(request.target, Cow::Borrowed(_)));
}
#[test]
fn frozen_frame_moves_and_outlives_the_mutable_input_buffer() {
    let mut input = bytes::BytesMut::from(
        b"GET /health HTTP/1.1\r\nHost: x\r\n\r\nGET /next HTTP/1.1\r\nHost: x\r\n\r\n".as_slice(),
    );
    let mut parser = RequestParser::default();
    let plan = parser.parse_indices(&input).unwrap().unwrap();
    let n = plan.consumed();
    let frame = plan.freeze(input.split_to(n).freeze()).unwrap();
    drop(input);
    let moved = std::thread::spawn(move || {
        let request = frame.request().unwrap();
        assert_eq!(request.path(), "/health");
        assert_eq!(request.method, Method::Get);
    });
    moved.join().unwrap();
}
#[test]
fn shrinking_or_mismatched_utf8_buffers_are_errors_without_panic() {
    let mut parser = RequestParser::default();
    assert!(
        parser
            .parse_indices(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 10\r\n\r\na")
            .unwrap()
            .is_none()
    );
    assert!(parser.parse_indices(&[]).is_err());
    let wire = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
    let plan = RequestParser::default()
        .parse_indices(wire)
        .unwrap()
        .unwrap();
    assert!(plan.request(&[]).is_err());
    let mut changed = wire.to_vec();
    changed[0] = 255;
    assert!(plan.request(&changed).is_err());
}
#[test]
fn smuggling_and_duplicate_length_policies_are_unchanged() {
    for wire in [
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n"
            .as_slice(),
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost : x\r\n\r\n",
        b"GET / HTTP/1.1\r\nHost: x\r\n folded: bad\r\n\r\n",
    ] {
        let before = baseline::parse_request(wire).map_err(|e| e.to_string());
        let after = parse_request(wire).map_err(|e| e.to_string());
        assert_eq!(before, after);
        assert!(after.is_err());
    }
}
