use std::io::{Read, Write};

use serde::Serialize;
use serde_json::{Value, json};

use crate::client::{Client, Response};
use crate::config::{self, Credentials, Env, Overrides, Store};

use super::{Error, Result, print_json};

/// The persistent flags defined on the root command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobalFlags {
    pub sandbox: bool,
    pub production: bool,
    pub account: String,
}

/// Provides lazily-built dependencies to commands (gh-style), so a command
/// only touches config/credentials/network when it actually runs.
pub struct Factory<'a> {
    pub input: &'a mut dyn Read,
    pub out: &'a mut dyn Write,
    pub version: String,
    pub flags: GlobalFlags,
    /// Reads the ASAAS_* variables; replaceable in tests.
    pub load_env: Box<dyn Fn() -> Env + 'a>,
}

impl<'a> Factory<'a> {
    /// A factory wired to the real process environment.
    pub fn new(input: &'a mut dyn Read, out: &'a mut dyn Write, version: &str) -> Self {
        Factory {
            input,
            out,
            version: version.to_string(),
            flags: GlobalFlags::default(),
            load_env: Box::new(config::load_env),
        }
    }

    /// The ASAAS_* environment variables.
    pub fn env(&self) -> Env {
        (self.load_env)()
    }

    /// The config file store.
    pub fn store(&self) -> Result<Store> {
        Ok(Store::default_for(&self.env())?)
    }

    /// The global flags as resolution overrides.
    pub fn overrides(&self) -> Overrides {
        Overrides {
            sandbox: self.flags.sandbox,
            production: self.flags.production,
            account: self.flags.account.clone(),
        }
    }

    /// Resolves which key/environment this invocation uses.
    pub fn credentials(&self) -> Result<Credentials> {
        let store = self.store()?;
        Ok(config::resolve(&self.env(), &store, &self.overrides())?)
    }

    /// An API client for the resolved credentials.
    pub fn client(&self) -> Result<Client> {
        let creds = self.credentials()?;
        Client::new(&creds.api_key, creds.environment)
            .and_then(|c| c.with_base_url(&creds.base_url))
            .map(|c| c.with_user_agent(format!("asaas-cli/{}", self.version)))
            .map_err(|e| Error::config(e.to_string()))
    }

    /// Writes v to stdout as JSON.
    pub fn print<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        print_json(self.out, v)
    }

    /// Writes an API response body to stdout. Empty bodies are rendered as
    /// {"status_code": N, "success": true} so stdout is always JSON.
    pub fn print_response(&mut self, resp: &Response) -> Result<()> {
        match &resp.body {
            Some(body) => self.print(body),
            None => self.print(&json!({"status_code": resp.status_code, "success": true})),
        }
    }

    /// Reads one line from stdin (without the line terminator).
    pub fn read_line(&mut self) -> std::io::Result<String> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.input.read(&mut byte) {
                Ok(0) => break,
                Ok(_) if byte[0] == b'\n' => break,
                Ok(_) => buf.push(byte[0]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if buf.is_empty() && byte[0] != b'\n' {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "EOF",
            ));
        }
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        String::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Builds a JSON object from key/value pairs, sorted by key (the shape used
/// for CLI-generated map outputs). Nested values keep their own order.
pub fn object<I>(pairs: I) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut entries: Vec<(&str, Value)> = pairs.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    Value::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
