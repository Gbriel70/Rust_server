//! Request views borrow validated bytes; stored parser state contains only indices.
use super::{
    headers::Headers as OwnedHeaders,
    request::{Method, Request as OwnedRequest, Version},
};
use std::{borrow::Cow, ops::Range};

#[derive(Debug, Clone)]
pub(crate) struct HeaderIndex {
    pub name: Range<usize>,
    pub value: Range<usize>,
}
#[derive(Debug, Clone)]
enum HeaderStorage<'a> {
    Indexed {
        head: &'a str,
        fields: Cow<'a, [HeaderIndex]>,
    },
    Original(&'a OwnedHeaders),
}
#[derive(Debug, Clone)]
pub struct Headers<'a>(HeaderStorage<'a>);
impl<'a> Headers<'a> {
    pub(crate) fn indexed(head: &'a str, fields: Cow<'a, [HeaderIndex]>) -> Self {
        Self(HeaderStorage::Indexed { head, fields })
    }
    pub fn get(&self, name: &str) -> Option<&str> {
        self.iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v)
    }
    pub fn has_token(&self, name: &str, token: &str) -> bool {
        !token.is_empty()
            && self
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                .flat_map(|(_, value)| value.split(','))
                .any(|value| value.trim_matches([' ', '\t']).eq_ignore_ascii_case(token))
    }
    pub fn iter(&self) -> HeaderIter<'_> {
        match &self.0 {
            HeaderStorage::Indexed { head, fields } => HeaderIter(HeaderIterKind::Indexed {
                head,
                fields: fields.iter(),
            }),
            HeaderStorage::Original(headers) => {
                HeaderIter(HeaderIterKind::Original(headers.raw().iter()))
            }
        }
    }
}
impl PartialEq for Headers<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}
pub struct HeaderIter<'a>(HeaderIterKind<'a>);
enum HeaderIterKind<'a> {
    Indexed {
        head: &'a str,
        fields: std::slice::Iter<'a, HeaderIndex>,
    },
    Original(std::slice::Iter<'a, (String, String)>),
}
impl<'a> Iterator for HeaderIter<'a> {
    type Item = (&'a str, &'a str);
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            HeaderIterKind::Indexed { head, fields } => {
                let field = fields.next()?;
                Some((
                    head.get(field.name.clone())?,
                    head.get(field.value.clone())?,
                ))
            }
            HeaderIterKind::Original(fields) => {
                fields.next().map(|(n, v)| (n.as_str(), v.as_str()))
            }
        }
    }
}
#[derive(Debug, PartialEq)]
pub struct Request<'a> {
    pub method: Method,
    pub target: Cow<'a, str>,
    pub version: Version,
    pub headers: Headers<'a>,
    pub body: Cow<'a, [u8]>,
}
impl<'a> Request<'a> {
    pub fn path(&self) -> &str {
        self.target
            .split_once('?')
            .map_or(self.target.as_ref(), |(path, _)| path)
    }
    pub fn query(&self) -> Option<&str> {
        self.target.split_once('?').map(|(_, query)| query)
    }
    pub fn keep_alive(&self) -> bool {
        !self.headers.has_token("Connection", "close")
            && (self.version == Version::Http11
                || self.headers.has_token("Connection", "keep-alive"))
    }
    pub fn into_owned_request(self) -> OwnedRequest {
        let mut headers = OwnedHeaders::new();
        for (name, value) in self.headers.iter() {
            headers.push(name.to_owned(), value.to_owned());
        }
        OwnedRequest {
            method: self.method,
            target: self.target.into_owned(),
            version: self.version,
            headers,
            body: self.body.into_owned(),
        }
    }
    /// Compatibility boundary for callers whose API requires an owned request.
    pub fn to_owned_request(&self) -> OwnedRequest {
        let mut headers = OwnedHeaders::new();
        for (name, value) in self.headers.iter() {
            headers.push(name.to_owned(), value.to_owned());
        }
        OwnedRequest {
            method: self.method,
            target: self.target.to_string(),
            version: self.version,
            headers,
            body: self.body.to_vec(),
        }
    }
}
impl<'a> From<&'a OwnedRequest> for Request<'a> {
    fn from(request: &'a OwnedRequest) -> Self {
        Self {
            method: request.method,
            target: Cow::Borrowed(&request.target),
            version: request.version,
            headers: Headers(HeaderStorage::Original(&request.headers)),
            body: Cow::Borrowed(&request.body),
        }
    }
}
