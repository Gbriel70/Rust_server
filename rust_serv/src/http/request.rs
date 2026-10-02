use std::str::FromStr;

// Struct request represents an HTTP request.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: Method,
    pub target: String,
    pub version: Version,
    pub headers: Headers,
    pub body: Vec<u8>,
}

// Enum method represents the HTTP method of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Options,
}

// Implementation of the FromStr trait for the Method enum, allowing conversion from a string to a Method variant.
impl FromStr for Method {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "GET" => Ok(Method::Get),
            "POST" => Ok(Method::Post),
            "PUT" => Ok(Method::Put),
            "DELETE" => Ok(Method::Delete),
            "HEAD" => Ok(Method::Head),
            "OPTIONS" => Ok(Method::Options),
            _ => Err(()),
        }
    }
}

// Enum version represents the HTTP version of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Http10,
    Http11
}

// Implementation of the FromStr trait for the Version enum, allowing conversion from a string to a Version variant.
impl FromStr for Version {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "HTTP/1.0" => Ok(Version::Http10),
            "HTTP/1.1" => Ok(Version::Http11),
            _ => Err(()),
        }
    }
}

// Struct headers represents the HTTP headers of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

// Implementation of methods for the Headers struct.
impl Headers {

    // Method to create a new instance of Headers.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    // Method to retrieve the value of a header by name, case-insensitive.
    pub fn get (&self, name: &str) -> Option<&str> {
        self.0.iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
    }

    // Method to add a new header to the collection.
    pub fn push(&mut self, name: String, value: String) {
        self.0.push((name, value));
    }

    // Method to return an iterator over the headers.
    pub fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.0.iter()
    }
}
