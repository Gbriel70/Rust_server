//! Storage and lookup for the headers of an HTTP message.

/// Ordered collection of HTTP headers.
///
/// Pairs are kept in the order in which they were received so repeated
/// headers are preserved and serialization remains predictable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    /// Creates an empty header collection.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Returns the first value whose name matches case-insensitively.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Appends a header to the collection while preserving its order.
    pub fn push(&mut self, name: String, value: String) {
        self.0.push((name, value));
    }

    /// Iterates over headers as `(name, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = &(String, String)> {
        self.0.iter()
    }
}
