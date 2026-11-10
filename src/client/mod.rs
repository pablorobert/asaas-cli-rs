//! A small, resource-agnostic HTTP client for the Asaas API v3. Every
//! endpoint is reachable through [`Client::request`] (and its get/post/put/
//! delete shortcuts); responses are returned as raw JSON values so callers
//! can pass them through untouched.

mod environment;
mod errors;
mod query;

use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

pub use environment::{Environment, PRODUCTION_BASE_URL, SANDBOX_BASE_URL, mask_key};
pub use errors::{ApiError, Error, ErrorDetail, NetworkError, ResponseError};
pub use query::{Query, path_escape};

/// Bounds a single HTTP round trip.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest "limit" accepted by Asaas list endpoints.
pub const MAX_PAGE_SIZE: usize = 100;

const DEFAULT_USER_AGENT: &str = "asaas-cli";
const MAX_RESPONSE_BYTES: u64 = 32 << 20;

/// Talks to one Asaas account in one environment.
#[derive(Debug, Clone)]
pub struct Client {
    base_url: Url,
    api_key: String,
    http: reqwest::blocking::Client,
    user_agent: String,
}

/// A successful (2xx) API response.
#[derive(Debug, Clone)]
pub struct Response {
    pub status_code: u16,
    pub headers: HeaderMap,
    /// The JSON body; None when the API returned no content.
    pub body: Option<Value>,
}

/// A request body.
#[derive(Debug, Clone)]
pub enum Body {
    /// Serialized as JSON, with object keys sorted.
    Json(Value),
    /// Sent verbatim (an already-encoded JSON document).
    Raw(Vec<u8>),
}

/// One file part of a multipart/form-data request.
#[derive(Debug, Clone)]
pub struct File {
    /// Form field name, e.g. "certificateFile".
    pub field: String,
    pub filename: String,
    pub content: Vec<u8>,
}

/// The envelope Asaas uses for every list endpoint.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Page {
    pub object: String,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
    #[serde(rename = "totalCount")]
    pub total_count: i64,
    pub limit: i64,
    pub offset: i64,
    pub data: Vec<Value>,
}

#[derive(Deserialize)]
struct RawPage {
    #[serde(default, rename = "hasMore")]
    has_more: bool,
    #[serde(default, rename = "totalCount")]
    total_count: i64,
    #[serde(default)]
    data: Option<Vec<Value>>,
}

impl Client {
    /// Builds a Client for the given key and environment.
    pub fn new(api_key: &str, env: Environment) -> Result<Self, Error> {
        if api_key.trim().is_empty() {
            return Err(Error::Invalid("asaasclient: API key is required".into()));
        }
        let base_url = Url::parse(env.base_url()).map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(Client {
            http: build_http(&base_url)?,
            base_url,
            api_key: api_key.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
        })
    }

    /// Overrides the environment base URL (proxies, mocks, tests).
    pub fn with_base_url(mut self, raw: &str) -> Result<Self, Error> {
        match Url::parse(raw) {
            Ok(u) if u.has_host() && !u.cannot_be_a_base() => {
                self.http = build_http(&u)?;
                self.base_url = u;
                Ok(self)
            }
            _ => Err(Error::Invalid(format!(
                "asaasclient: invalid base URL {raw:?}"
            ))),
        }
    }

    /// Sets the User-Agent header (required by Asaas for newer accounts).
    pub fn with_user_agent(mut self, ua: impl Into<String>) -> Self {
        self.user_agent = ua.into();
        self
    }

    /// The base URL requests are sent to.
    pub fn base_url(&self) -> &str {
        self.base_url.as_str()
    }

    /// Sends a request to path (relative to the /v3 base, e.g. "/customers").
    pub fn request(
        &self,
        method: &str,
        path: &str,
        query: Option<&Query>,
        body: Option<Body>,
    ) -> Result<Response, Error> {
        let url = self.resolve(path, query)?;
        let payload =
            match body {
                None => None,
                Some(Body::Raw(b)) => Some(b),
                Some(Body::Json(v)) => Some(serde_json::to_vec(&sort_keys(v)).map_err(|e| {
                    Error::Invalid(format!("asaasclient: encoding request body: {e}"))
                })?),
            };
        self.send(method, url, payload, "application/json")
    }

    /// Sends a multipart/form-data POST, used by the few Asaas endpoints that
    /// accept file uploads (e.g. /fiscalInfo).
    pub fn post_multipart(
        &self,
        path: &str,
        fields: &BTreeMap<String, String>,
        files: &[File],
    ) -> Result<Response, Error> {
        let url = self.resolve(path, None)?;
        let boundary = random_boundary();
        let mut buf = Vec::new();
        for (k, v) in fields {
            buf.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n",
                    escape_quotes(k)
                )
                .as_bytes(),
            );
            buf.extend_from_slice(v.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        for f in files {
            buf.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: application/octet-stream\r\n\r\n",
                    escape_quotes(&f.field),
                    escape_quotes(&f.filename)
                )
                .as_bytes(),
            );
            buf.extend_from_slice(&f.content);
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        self.send(
            "POST",
            url,
            Some(buf),
            &format!("multipart/form-data; boundary={boundary}"),
        )
    }

    fn send(
        &self,
        method: &str,
        url: Url,
        body: Option<Vec<u8>>,
        content_type: &str,
    ) -> Result<Response, Error> {
        let m = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| Error::Invalid(format!("asaasclient: invalid method {method:?}")))?;
        let mut req = self
            .http
            .request(m, url.clone())
            .header("access_token", &self.api_key)
            .header(CONTENT_TYPE, content_type)
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, &self.user_agent);
        if let Some(b) = body {
            req = req.body(b);
        }

        let net = |e: &reqwest::Error| {
            Error::Network(NetworkError::from_reqwest(method, url.as_str(), e))
        };
        let resp = req.send().map_err(|e| net(&e))?;
        let status = resp.status().as_u16();
        let headers = resp.headers().clone();

        let mut raw = Vec::new();
        if let Err(e) = resp.take(MAX_RESPONSE_BYTES).read_to_end(&mut raw) {
            return Err(Error::Network(NetworkError {
                method: method.to_string(),
                url: url.to_string(),
                message: e.to_string(),
                timeout: e.kind() == std::io::ErrorKind::TimedOut,
            }));
        }
        if !(200..=299).contains(&status) {
            return Err(Error::Api(ApiError::new(method, url.path(), status, &raw)));
        }

        let trimmed = raw.trim_ascii();
        if trimmed.is_empty() {
            return Ok(Response {
                status_code: status,
                headers,
                body: None,
            });
        }
        match serde_json::from_slice::<Value>(trimmed) {
            Ok(v) => Ok(Response {
                status_code: status,
                headers,
                body: Some(v),
            }),
            Err(_) => Err(Error::Response(ResponseError {
                status_code: status,
                message: "response body is not valid JSON".into(),
                body: truncate(&String::from_utf8_lossy(trimmed), 2048),
            })),
        }
    }

    /// Issues a GET request.
    pub fn get(&self, path: &str, query: Option<&Query>) -> Result<Response, Error> {
        self.request("GET", path, query, None)
    }

    /// Issues a POST request with a JSON body.
    pub fn post(&self, path: &str, body: Value) -> Result<Response, Error> {
        self.request("POST", path, None, Some(Body::Json(body)))
    }

    /// Issues a PUT request with a JSON body.
    pub fn put(&self, path: &str, body: Value) -> Result<Response, Error> {
        self.request("PUT", path, None, Some(Body::Json(body)))
    }

    /// Issues a DELETE request.
    pub fn delete(&self, path: &str) -> Result<Response, Error> {
        self.request("DELETE", path, None, None)
    }

    /// Walks every page of a list endpoint (starting at the "offset" in
    /// query, if any) and returns a single Page holding all items.
    pub fn list_all(&self, path: &str, query: &Query) -> Result<Page, Error> {
        let mut q = query.clone();
        let start: i64 = q.get("offset").and_then(|s| s.parse().ok()).unwrap_or(0);
        q.set("limit", MAX_PAGE_SIZE.to_string());

        let mut all = Page {
            object: "list".into(),
            offset: start,
            ..Page::default()
        };
        let mut offset = start;
        loop {
            q.set("offset", offset.to_string());
            let resp = self.get(path, Some(&q))?;
            let not_page = |msg: String, body: &Option<Value>| {
                Error::Response(ResponseError {
                    status_code: resp.status_code,
                    message: format!("response is not a list page: {msg}"),
                    body: body
                        .as_ref()
                        .map(|b| truncate(&b.to_string(), 2048))
                        .unwrap_or_default(),
                })
            };
            let page: RawPage = match &resp.body {
                None => return Err(not_page("empty body".into(), &resp.body)),
                Some(v) => serde_json::from_value(v.clone())
                    .map_err(|e| not_page(e.to_string(), &resp.body))?,
            };
            let data = page.data.unwrap_or_default();
            let n = data.len();
            all.data.extend(data);
            all.total_count = page.total_count;
            if !page.has_more || n == 0 {
                break;
            }
            offset += n as i64;
        }
        all.limit = all.data.len() as i64;
        Ok(all)
    }

    /// Joins path onto the base URL. Only relative API paths are accepted so
    /// the access token is never sent to another host.
    fn resolve(&self, path: &str, query: Option<&Query>) -> Result<Url, Error> {
        let p = path.trim().trim_start_matches('/');
        let p = p.strip_prefix("v3/").unwrap_or(p);

        // A ':' before any '/', '?' or '#' means a scheme ("https://...").
        if let Some(i) = p.find([':', '/', '?', '#'])
            && p.as_bytes()[i] == b':'
        {
            return Err(Error::Invalid(format!(
                "asaasclient: path {path:?} must be relative to the API base (e.g. /customers)"
            )));
        }
        let path_part = p.split(['?', '#']).next().unwrap_or("");
        let dot_dot = path_part.split('/').any(|seg| {
            matches!(
                seg.to_ascii_lowercase().as_str(),
                ".." | ".%2e" | "%2e." | "%2e%2e"
            )
        });
        if dot_dot {
            return Err(Error::Invalid(format!(
                "asaasclient: path {path:?} must not contain '..'"
            )));
        }

        let mut base = self.base_url.clone();
        let base_path = format!("{}/", base.path().trim_end_matches('/'));
        base.set_path(&base_path);
        base.set_query(None);
        base.set_fragment(None);
        let mut u = base
            .join(p)
            .map_err(|e| Error::Invalid(format!("asaasclient: invalid path {path:?}: {e}")))?;
        u.set_fragment(None);

        let mut q: Query = u
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if let Some(extra) = query {
            for (k, v) in extra.iter() {
                q.add(k, v);
            }
        }
        if q.is_empty() {
            u.set_query(None);
        } else {
            u.set_query(Some(&q.encode()));
        }
        Ok(u)
    }
}

/// The HTTP client. HTTP(S)_PROXY from the environment is honored, except
/// for loopback hosts (local mocks), which are always reached directly.
fn build_http(base: &Url) -> Result<reqwest::blocking::Client, Error> {
    let loopback = match base.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    };
    let mut builder = reqwest::blocking::Client::builder().timeout(DEFAULT_TIMEOUT);
    if loopback {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|e| Error::Invalid(format!("asaasclient: building HTTP client: {e}")))
}

/// Returns v with every object's keys sorted, recursively.
pub fn sort_keys(v: Value) -> Value {
    match v {
        Value::Object(m) => {
            let mut entries: Vec<(String, Value)> = m.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, sort_keys(v)))
                    .collect(),
            )
        }
        Value::Array(a) => Value::Array(a.into_iter().map(sort_keys).collect()),
        other => other,
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn escape_quotes(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn random_boundary() -> String {
    let state = std::collections::hash_map::RandomState::new();
    let mut a = state.build_hasher();
    a.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let mut b = state.build_hasher();
    b.write_u64(a.finish() ^ u64::from(std::process::id()));
    format!("{:016x}{:016x}", a.finish(), b.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    /// Serves each request with handler on a local port; returns the base URL.
    fn serve<F>(handler: F) -> (String, Arc<tiny_http::Server>)
    where
        F: Fn(&mut tiny_http::Request, String) -> (u16, String) + Send + 'static,
    {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let url = format!("http://{}/v3", server.server_addr().to_ip().unwrap());
        let s = server.clone();
        thread::spawn(move || {
            for mut req in s.incoming_requests() {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                let (status, out) = handler(&mut req, body);
                let _ = req.respond(tiny_http::Response::from_string(out).with_status_code(status));
            }
        });
        (url, server)
    }

    fn header(req: &tiny_http::Request, name: &str) -> String {
        req.headers()
            .iter()
            .find(|h| h.field.to_string().eq_ignore_ascii_case(name))
            .map(|h| h.value.to_string())
            .unwrap_or_default()
    }

    fn test_client(url: &str) -> Client {
        Client::new("$aact_hmlg_secret", Environment::Sandbox)
            .unwrap()
            .with_base_url(url)
            .unwrap()
            .with_user_agent("test-agent")
    }

    #[test]
    fn request_sends_headers_and_body() {
        let (url, srv) = serve(|req, body| {
            let ok = header(req, "access_token") == "$aact_hmlg_secret"
                && header(req, "Content-Type") == "application/json"
                && header(req, "User-Agent") == "test-agent"
                && req.method() == &tiny_http::Method::Post
                && req.url() == "/v3/customers"
                && body == r#"{"name":"Ana"}"#;
            if ok {
                (200, r#"{"id":"cus_1","value":12345678901234567890}"#.into())
            } else {
                (418, format!("unexpected request: {} {body}", req.url()))
            }
        });
        let resp = test_client(&url)
            .post("/customers", serde_json::json!({"name": "Ana"}))
            .unwrap();
        // Numbers are preserved byte-for-byte (no float64 rounding).
        assert_eq!(
            serde_json::to_string(&resp.body.unwrap()).unwrap(),
            r#"{"id":"cus_1","value":12345678901234567890}"#
        );
        srv.unblock();
    }

    #[test]
    fn resolve_paths() {
        let c = Client::new("k", Environment::Production).unwrap();
        let cases = [
            ("/customers", "https://api.asaas.com/v3/customers"),
            ("customers", "https://api.asaas.com/v3/customers"),
            (
                "/v3/customers/cus_1",
                "https://api.asaas.com/v3/customers/cus_1",
            ),
            (
                "/payments?status=PAID",
                "https://api.asaas.com/v3/payments?status=PAID",
            ),
        ];
        for (input, want) in cases {
            assert_eq!(
                c.resolve(input, None).unwrap().as_str(),
                want,
                "resolve({input:?})"
            );
        }
        for bad in [
            "https://evil.example/v3/customers",
            "/customers/../../x",
            "/a/%2e%2e/x",
        ] {
            assert!(
                c.resolve(bad, None).is_err(),
                "resolve({bad:?}) should fail"
            );
        }
        // Leading slashes are stripped, so a protocol-relative path cannot switch hosts.
        let u = c.resolve("//evil.example/x", None).unwrap();
        assert_eq!(u.host_str(), Some("api.asaas.com"));

        let q: Query = [("limit", "5"), ("name", "Ana")].into_iter().collect();
        assert_eq!(
            c.resolve("/customers?offset=1", Some(&q)).unwrap().as_str(),
            "https://api.asaas.com/v3/customers?limit=5&name=Ana&offset=1"
        );
    }

    #[test]
    fn api_error_parsing() {
        let (url, srv) = serve(|_, _| {
            (
                400,
                r#"{"errors":[{"code":"invalid_cpfCnpj","description":"CPF inválido"}]}"#.into(),
            )
        });
        match test_client(&url).get("/customers", None) {
            Err(Error::Api(e)) => {
                assert_eq!(e.status_code, 400);
                assert_eq!(e.errors.len(), 1);
                assert_eq!(e.errors[0].code, "invalid_cpfCnpj");
                assert_eq!(e.path, "/v3/customers");
            }
            other => panic!("want Api error, got {other:?}"),
        }
        srv.unblock();
    }

    #[test]
    fn non_json_error_body_is_kept() {
        let (url, srv) = serve(|_, _| (502, "<html>bad gateway</html>".into()));
        match test_client(&url).get("/customers", None) {
            Err(Error::Api(e)) => assert_eq!(e.body, "<html>bad gateway</html>"),
            other => panic!("want Api error, got {other:?}"),
        }
        srv.unblock();
    }

    #[test]
    fn invalid_json_success_is_response_error() {
        let (url, srv) = serve(|_, _| (200, "not json".into()));
        assert!(matches!(
            test_client(&url).get("/customers", None),
            Err(Error::Response(_))
        ));
        srv.unblock();
    }

    #[test]
    fn network_error() {
        let c = test_client("http://127.0.0.1:1/v3");
        assert!(matches!(c.get("/customers", None), Err(Error::Network(_))));
    }

    #[test]
    fn list_all_follows_pagination() {
        const TOTAL: usize = 250;
        let (url, srv) = serve(|req, _| {
            let u = Url::parse(&format!("http://x{}", req.url())).unwrap();
            let q: Query = u
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let offset: usize = q.get("offset").unwrap().parse().unwrap();
            let limit: usize = q.get("limit").unwrap().parse().unwrap();
            if limit != MAX_PAGE_SIZE || q.get("name") != Some("x") {
                return (418, format!("bad query {}", req.url()));
            }
            let data: Vec<Value> = (offset..TOTAL.min(offset + limit))
                .map(|i| serde_json::json!({"n": i}))
                .collect();
            let page = serde_json::json!({
                "object": "list", "hasMore": offset + limit < TOTAL, "totalCount": TOTAL,
                "limit": limit, "offset": offset, "data": data,
            });
            (200, page.to_string())
        });
        let q: Query = [("name", "x"), ("limit", "5")].into_iter().collect();
        let page = test_client(&url).list_all("/customers", &q).unwrap();
        assert_eq!(page.data.len(), TOTAL);
        assert!(!page.has_more);
        assert_eq!(page.total_count, TOTAL as i64);
        assert_eq!(page.limit, TOTAL as i64);
        srv.unblock();
    }

    #[test]
    fn sort_keys_is_recursive() {
        let v = serde_json::json!({"b": 1, "a": {"d": 1, "c": [{"z": 1, "y": 2}]}});
        assert_eq!(
            serde_json::to_string(&sort_keys(v)).unwrap(),
            r#"{"a":{"c":[{"y":2,"z":1}],"d":1},"b":1}"#
        );
    }
}
