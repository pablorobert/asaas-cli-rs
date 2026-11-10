use clap::{ArgMatches, Command};
use serde_json::{Map, Value};

use crate::client::{Body, Query, path_escape};

use super::{Factory, Field, Result, collect_query, validate_pagination};

/// A parent command (e.g. "customer") that only hosts subcommands. Running
/// it alone prints its help; an unknown subcommand is a usage error.
pub fn new_group(
    name: &'static str,
    usage: &'static str,
    short: &'static str,
    long: impl Into<String>,
) -> Command {
    super::new_command(name, usage, short, long)
}

/// Prints the long help of a group invoked without a subcommand.
pub fn print_group_help(f: &mut Factory, cmd: &mut Command) -> Result<()> {
    let help = cmd.render_long_help().to_string();
    f.out
        .write_all(help.as_bytes())
        .map_err(|e| super::Error::internal(format!("writing help: {e}")))
}

/// Joins a collection path with escaped path segments,
/// e.g. resource_path("/customers", &[id, "restore"]).
pub fn resource_path(base: &str, segments: &[&str]) -> String {
    let mut parts = vec![base.trim_end_matches('/').to_string()];
    parts.extend(segments.iter().map(|s| path_escape(s)));
    parts.join("/")
}

/// Runs a GET on a list endpoint using filters declared as Fields. With
/// all=true every page is fetched and merged into one list.
pub fn run_list(
    f: &mut Factory,
    m: &ArgMatches,
    path: &str,
    filters: &[Field],
    all: bool,
) -> Result<()> {
    validate_pagination(m)?;
    let query = collect_query(m, filters)?;
    let client = f.client()?;
    if all {
        let page = client.list_all(path, &query)?;
        return f.print(&page);
    }
    let resp = client.get(path, Some(&query))?;
    f.print_response(&resp)
}

/// Sends one request with the resolved client and prints the response.
pub fn run_request(
    f: &mut Factory,
    method: &str,
    path: &str,
    query: Option<&Query>,
    body: Option<Map<String, Value>>,
) -> Result<()> {
    let client = f.client()?;
    let resp = client.request(
        method,
        path,
        query,
        body.map(|b| Body::Json(Value::Object(b))),
    )?;
    f.print_response(&resp)
}
