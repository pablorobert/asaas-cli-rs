//! asaas: a non-interactive command-line client for the Asaas API v3,
//! designed to be driven by AI agents and automation scripts.
//!
//! - [`client`]: small, resource-agnostic HTTP client for the API.
//! - [`config`]: multi-account config file and `ASAAS_*` variables.
//! - [`cmdutil`]: plumbing shared by every command (factory, JSON output,
//!   error rendering, declarative flags).
//! - [`cmd`]: the command tree.

pub mod client;
pub mod cmd;
pub mod cmdutil;
pub mod config;

/// CLI version, set at build time through `ASAAS_VERSION` (default "dev").
pub const VERSION: &str = env!("ASAAS_CLI_VERSION");

/// Runs the CLI with the process arguments and returns the exit code.
pub fn execute() -> i32 {
    cmd::execute(VERSION)
}
