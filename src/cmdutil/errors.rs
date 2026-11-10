use std::fmt;
use std::io::Write;

use serde_json::{Map, Value, json};

use crate::client;
use crate::config::ConfigError;

/// Classifies an error in the JSON written to stderr ("type" field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Usage,
    Auth,
    Config,
    Api,
    Network,
    Response,
    Internal,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Usage => "usage_error",
            Kind::Auth => "auth_error",
            Kind::Config => "config_error",
            Kind::Api => "api_error",
            Kind::Network => "network_error",
            Kind::Response => "response_error",
            Kind::Internal => "internal_error",
        }
    }

    /// The process exit code for this kind.
    pub fn exit_code(self) -> i32 {
        match self {
            Kind::Usage => EXIT_USAGE,
            Kind::Auth | Kind::Config => EXIT_AUTH,
            Kind::Network => EXIT_NETWORK,
            Kind::Api | Kind::Response | Kind::Internal => EXIT_ERROR,
        }
    }
}

// Process exit codes, documented in the root command help.
pub const EXIT_OK: i32 = 0;
/// api_error, response_error, internal_error
pub const EXIT_ERROR: i32 = 1;
/// usage_error
pub const EXIT_USAGE: i32 = 2;
/// auth_error, config_error
pub const EXIT_AUTH: i32 = 3;
/// network_error
pub const EXIT_NETWORK: i32 = 4;

/// Every error a command can return.
#[derive(Debug, Clone)]
pub enum Error {
    /// A classified CLI error.
    Cli {
        kind: Kind,
        message: String,
        hint: Option<String>,
    },
    Client(client::Error),
    Config(ConfigError),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// A usage_error (invalid flags/arguments).
    pub fn usage(message: impl Into<String>) -> Self {
        Error::Cli {
            kind: Kind::Usage,
            message: message.into(),
            hint: None,
        }
    }

    /// A config_error.
    pub fn config(message: impl Into<String>) -> Self {
        Error::Cli {
            kind: Kind::Config,
            message: message.into(),
            hint: None,
        }
    }

    /// An internal_error.
    pub fn internal(message: impl Into<String>) -> Self {
        Error::Cli {
            kind: Kind::Internal,
            message: message.into(),
            hint: None,
        }
    }
}

/// Builds a usage_error with format! syntax.
#[macro_export]
macro_rules! usage_err {
    ($($arg:tt)*) => { $crate::cmdutil::Error::usage(format!($($arg)*)) };
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cli { message, .. } => f.write_str(message),
            Error::Client(e) => e.fmt(f),
            Error::Config(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<client::Error> for Error {
    fn from(e: client::Error) -> Self {
        Error::Client(e)
    }
}

impl From<ConfigError> for Error {
    fn from(e: ConfigError) -> Self {
        Error::Config(e)
    }
}

const AUTH_HINT: &str = "set ASAAS_API_KEY, or run: asaas auth add --name <name> --key <api-key> --sandbox|--production";

/// Renders err as {"error": {...}} JSON on w and returns the exit code.
/// command_path ("asaas customer list") is reported for usage errors.
pub fn handle_error(w: &mut dyn Write, command_path: Option<&str>, err: &Error) -> i32 {
    let mut p = Map::new();
    p.insert("message".into(), json!(err.to_string()));

    let kind = match err {
        Error::Client(client::Error::Api(e)) => {
            p.insert("status_code".into(), json!(e.status_code));
            p.insert("method".into(), json!(e.method));
            p.insert("path".into(), json!(e.path));
            if !e.errors.is_empty() {
                p.insert("errors".into(), json!(e.errors));
            }
            if !e.body.is_empty() {
                p.insert("body".into(), json!(e.body));
            }
            if e.status_code == 401 {
                p.insert(
                    "hint".into(),
                    json!("the API key was rejected; check that it belongs to the selected environment (--sandbox / --production)"),
                );
            }
            Kind::Api
        }
        Error::Client(client::Error::Network(e)) => {
            p.insert("method".into(), json!(e.method));
            p.insert("url".into(), json!(e.url));
            p.insert("timeout".into(), json!(e.timeout));
            Kind::Network
        }
        Error::Client(client::Error::Response(e)) => {
            p.insert("status_code".into(), json!(e.status_code));
            if !e.body.is_empty() {
                p.insert("body".into(), json!(e.body));
            }
            Kind::Response
        }
        // Bad input handed to the client (e.g. an absolute URL as path).
        Error::Client(client::Error::Invalid(_)) => Kind::Usage,
        Error::Config(ConfigError::NoCredentials) => {
            p.insert("hint".into(), json!(AUTH_HINT));
            Kind::Auth
        }
        Error::Config(ConfigError::AccountNotFound { available, .. }) => {
            p.insert("available_accounts".into(), json!(available));
            Kind::Config
        }
        Error::Config(ConfigError::InvalidName(_)) => Kind::Usage,
        Error::Config(_) => Kind::Config,
        Error::Cli { kind, hint, .. } => {
            if let Some(h) = hint {
                p.insert("hint".into(), json!(h));
            }
            *kind
        }
    };
    p.insert("type".into(), json!(kind.as_str()));

    if kind == Kind::Usage
        && let Some(path) = command_path
    {
        p.insert("command".into(), json!(path));
        p.entry("hint")
            .or_insert_with(|| json!(format!("run '{path} --help' for usage")));
    }

    let doc = json!({ "error": client::sort_keys(Value::Object(p)) });
    if super::print_json(w, &doc).is_err() {
        let _ = writeln!(
            w,
            "{{\"error\":{{\"type\":\"internal_error\",\"message\":{}}}}}",
            json!(err.to_string())
        );
    }
    kind.exit_code()
}
