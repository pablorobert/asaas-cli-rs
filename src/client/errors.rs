use std::fmt;

use serde::{Deserialize, Serialize};

/// One entry of the "errors" array returned by Asaas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorDetail {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub description: String,
}

/// Returned when Asaas answers with a non-2xx status code.
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status_code: u16,
    pub method: String,
    pub path: String,
    pub errors: Vec<ErrorDetail>,
    /// The raw response body when it is not the standard
    /// {"errors": [...]} envelope.
    pub body: String,
}

impl ApiError {
    pub(crate) fn new(method: &str, path: &str, status_code: u16, body: &[u8]) -> Self {
        #[derive(Deserialize)]
        struct Envelope {
            #[serde(default)]
            errors: Vec<ErrorDetail>,
        }
        let mut e = ApiError {
            status_code,
            method: method.to_string(),
            path: path.to_string(),
            errors: Vec::new(),
            body: String::new(),
        };
        match serde_json::from_slice::<Envelope>(body) {
            Ok(env) if !env.errors.is_empty() => e.errors = env.errors,
            _ => e.body = String::from_utf8_lossy(body).trim().to_string(),
        }
        e
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "asaas API {} {} returned HTTP {}",
            self.method, self.path, self.status_code
        )?;
        if !self.errors.is_empty() {
            let parts: Vec<&str> = self.errors.iter().map(|d| d.description.as_str()).collect();
            write!(f, ": {}", parts.join("; "))?;
        }
        Ok(())
    }
}

/// Wraps transport failures (DNS, TLS, timeouts, resets...).
#[derive(Debug, Clone)]
pub struct NetworkError {
    pub method: String,
    pub url: String,
    pub message: String,
    /// Whether the failure was caused by a timeout.
    pub timeout: bool,
}

impl NetworkError {
    pub(crate) fn from_reqwest(method: &str, url: &str, err: &reqwest::Error) -> Self {
        // reqwest's Display is terse; append the cause chain (e.g. the OS error).
        let mut message = err.to_string();
        let mut source = std::error::Error::source(err);
        while let Some(s) = source {
            message.push_str(": ");
            message.push_str(&s.to_string());
            source = s.source();
        }
        NetworkError {
            method: method.to_string(),
            url: url.to_string(),
            message,
            timeout: err.is_timeout(),
        }
    }
}

impl fmt::Display for NetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "network error on {} {}: {}",
            self.method, self.url, self.message
        )
    }
}

/// Returned when a successful response cannot be decoded.
#[derive(Debug, Clone)]
pub struct ResponseError {
    pub status_code: u16,
    pub message: String,
    pub body: String,
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unexpected response (HTTP {}): {}",
            self.status_code, self.message
        )
    }
}

/// Every failure the client can report.
#[derive(Debug, Clone)]
pub enum Error {
    Api(ApiError),
    Network(NetworkError),
    Response(ResponseError),
    /// Invalid input given to the client (bad path, base URL, body...).
    Invalid(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Api(e) => e.fmt(f),
            Error::Network(e) => e.fmt(f),
            Error::Response(e) => e.fmt(f),
            Error::Invalid(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}
