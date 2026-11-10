use std::collections::BTreeMap;

/// Multi-valued URL query parameters. Keys are kept sorted, so the encoded
/// query string is deterministic ("limit=5&name=Ana").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query(BTreeMap<String, Vec<String>>);

impl Query {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a value to key.
    pub fn add(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.entry(key.into()).or_default().push(value.into());
    }

    /// Replaces every value of key.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.0.insert(key.into(), vec![value.into()]);
    }

    /// The first value of key, if any.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.first()).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .flat_map(|(k, vs)| vs.iter().map(move |v| (k.as_str(), v.as_str())))
    }

    /// Encodes as "k=v&k2=v2", sorted by key, escaping like a form
    /// (space becomes '+', only A-Z a-z 0-9 - _ . ~ stay literal).
    pub fn encode(&self) -> String {
        self.iter()
            .map(|(k, v)| format!("{}={}", query_escape(k), query_escape(v)))
            .collect::<Vec<_>>()
            .join("&")
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for Query {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut q = Query::new();
        for (k, v) in iter {
            q.add(k, v);
        }
        q
    }
}

fn query_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Escapes one URL path segment: unreserved characters and `$&+:=@` stay
/// literal, everything else (including '/') is percent-encoded.
pub fn path_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~'
            | b'$'
            | b'&'
            | b'+'
            | b':'
            | b'='
            | b'@' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_sorts_and_escapes() {
        let mut q = Query::new();
        q.add("status", "AUTHORIZED");
        q.add("effectiveDate[ge]", "2026-10-01");
        q.add("name", "Ana Maria");
        q.add("name", "a&b");
        assert_eq!(
            q.encode(),
            "effectiveDate%5Bge%5D=2026-10-01&name=Ana+Maria&name=a%26b&status=AUTHORIZED"
        );
        q.set("name", "x");
        assert_eq!(q.get("name"), Some("x"));
    }

    #[test]
    fn path_escape_segment() {
        assert_eq!(path_escape("cus_1"), "cus_1");
        assert_eq!(path_escape("a/b c?"), "a%2Fb%20c%3F");
    }
}
