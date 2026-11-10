use clap::Command;
use serde_json::json;

use crate::cmdutil::{self, Factory, Result, object};

pub fn command() -> Command {
    cmdutil::new_command(
        "version",
        "asaas version",
        "Print the CLI version as JSON",
        r#"Print build information as JSON:
  {"arch": "...", "os": "...", "rust_version": "...", "version": "<semver or dev>"}"#,
    )
}

pub fn run(f: &mut Factory) -> Result<()> {
    let out = object([
        ("version", json!(f.version)),
        ("rust_version", json!(env!("ASAAS_CLI_RUST_VERSION"))),
        ("os", json!(std::env::consts::OS)),
        ("arch", json!(std::env::consts::ARCH)),
    ]);
    f.print(&out)
}
