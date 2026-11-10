//! The asaas command tree.

mod api;
mod auth;
mod customer;
mod fiscalinfo;
mod invoice;
mod version;

use std::io::Write;

use clap::error::ErrorKind;
use clap::{ArgMatches, Command};

use crate::cmdutil::{
    self, EXIT_OK, Error, Factory, GlobalFlags, Result, bool_arg, get_bool, get_string, string_arg,
};

const ROOT_LONG: &str = r#"asaas is a non-interactive command-line client for the Asaas API v3,
designed to be driven by AI agents and automation scripts.

OUTPUT CONTRACT
  - Success: exactly one indented JSON document on stdout (the raw Asaas
    response body, or a CLI-generated object for local commands).
  - Failure: one JSON document on stderr, shaped as
      {"error": {"type": "<kind>", "message": "...", ...}}
    where type is one of: usage_error, auth_error, config_error, api_error,
    network_error, response_error, internal_error. api_error also carries
    status_code and the Asaas "errors" array ([{code, description}]).
  - No colors, no prompts, no pagers.

EXIT CODES
  0 success | 1 api/response/internal error | 2 usage error
  3 auth/config error | 4 network error

AUTHENTICATION (first match wins)
  1. --account <name>      use that account from the config file.
  2. ASAAS_API_KEY         use this key; the config file is ignored.
  3. active_account        from ~/.config/asaas/config.json (see 'asaas auth').

ENVIRONMENT (sandbox vs production)
  --sandbox / --production force the environment for one invocation.
  Otherwise: the account's "environment" (config file), or for ASAAS_API_KEY:
  ASAAS_ENV, then the key prefix ($aact_hmlg_ = sandbox, $aact_prod_ =
  production), then production.
    production: https://api.asaas.com/v3
    sandbox:    https://api-sandbox.asaas.com/v3

ENVIRONMENT VARIABLES
  ASAAS_API_KEY     API key (takes precedence over the config file)
  ASAAS_ENV         "sandbox" or "production", used with ASAAS_API_KEY
  ASAAS_CONFIG_DIR  config directory (default: ~/.config/asaas)
  ASAAS_BASE_URL    override the API base URL (proxies, mocks)

Any endpoint without a dedicated command is reachable through 'asaas api'."#;

/// Builds the full command tree.
pub fn root_command() -> Command {
    let root = cmdutil::new_command(
        "asaas",
        "asaas <command> <subcommand> [flags]",
        "Asaas API v3 CLI with JSON output, built for AI agents and automation",
        ROOT_LONG,
    );
    cmdutil::with_examples(
        root,
        r#"  asaas auth add --name sandbox-dev --key "$KEY" --sandbox
  asaas customer list --limit 5
  ASAAS_API_KEY="$KEY" asaas customer list --sandbox --name "Maria"
  asaas invoice list --status AUTHORIZED
  asaas api /payments --query status=PENDING"#,
    )
    .disable_version_flag(true)
    .arg(
        bool_arg("sandbox", "Force the sandbox environment (https://api-sandbox.asaas.com/v3) for this invocation, overriding the account default; with 'auth add', marks the account as sandbox [type: boolean]")
            .global(true)
            .conflicts_with("production"),
    )
    .arg(
        bool_arg("production", "Force the production environment (https://api.asaas.com/v3) for this invocation, overriding the account default; with 'auth add', marks the account as production [type: boolean]")
            .global(true),
    )
    .arg(
        string_arg("account", "Use this config-file account (nickname) for this invocation instead of the active one; takes precedence over ASAAS_API_KEY [type: string]")
            .global(true),
    )
    .subcommands([
        auth::command(),
        customer::command(),
        invoice::command(),
        fiscalinfo::command(),
        api::command(),
        version::command(),
    ])
}

/// Runs the CLI with the process arguments and returns the exit code.
pub fn execute(version: &str) -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut stdin = std::io::stdin().lock();
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut f = Factory::new(&mut stdin, &mut stdout, version);
    run(&mut f, &mut stderr, &args)
}

/// Executes args against a Factory, rendering any error as JSON on err_out,
/// and returns the exit code.
pub fn run<S: AsRef<str>>(f: &mut Factory, err_out: &mut dyn Write, args: &[S]) -> i32 {
    let mut root = root_command();
    let argv = std::iter::once("asaas").chain(args.iter().map(AsRef::as_ref));
    let m = match root.try_get_matches_from_mut(argv) {
        Ok(m) => m,
        Err(e) => {
            if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                return match f.out.write_all(e.render().to_string().as_bytes()) {
                    Ok(()) => EXIT_OK,
                    Err(io) => {
                        cmdutil::handle_error(err_out, None, &Error::internal(io.to_string()))
                    }
                };
            }
            let path = path_from_args(&root, args);
            return cmdutil::handle_error(err_out, Some(&path), &Error::usage(clap_message(&e)));
        }
    };

    let path = path_from_matches(&m);
    f.flags = global_flags(leaf(&m));
    match dispatch(f, &mut root, &m) {
        Ok(()) => EXIT_OK,
        Err(e) => cmdutil::handle_error(err_out, Some(&path), &e),
    }
}

fn dispatch(f: &mut Factory, root: &mut Command, m: &ArgMatches) -> Result<()> {
    let Some((name, sub)) = m.subcommand() else {
        return cmdutil::print_group_help(f, root);
    };
    let cmd = root
        .find_subcommand_mut(name)
        .ok_or_else(|| Error::internal(format!("unknown command {name}")))?;
    match name {
        "auth" => auth::run(f, cmd, sub),
        "customer" => customer::run(f, cmd, sub),
        "invoice" => invoice::run(f, cmd, sub),
        "fiscal-info" => fiscalinfo::run(f, cmd, sub),
        "api" => api::run(f, sub),
        "version" => version::run(f),
        other => Err(Error::internal(format!("unhandled command {other}"))),
    }
}

fn leaf(m: &ArgMatches) -> &ArgMatches {
    let mut cur = m;
    while let Some((_, sub)) = cur.subcommand() {
        cur = sub;
    }
    cur
}

fn global_flags(m: &ArgMatches) -> GlobalFlags {
    GlobalFlags {
        sandbox: get_bool(m, "sandbox"),
        production: get_bool(m, "production"),
        account: get_string(m, "account"),
    }
}

/// "asaas customer list" for the matched subcommand chain.
fn path_from_matches(m: &ArgMatches) -> String {
    let mut path = String::from("asaas");
    let mut cur = m;
    while let Some((name, sub)) = cur.subcommand() {
        path.push(' ');
        path.push_str(name);
        cur = sub;
    }
    path
}

/// Best-effort command path for arguments that failed to parse: follows the
/// leading words that name subcommands.
fn path_from_args<S: AsRef<str>>(root: &Command, args: &[S]) -> String {
    let mut path = String::from("asaas");
    let mut cur = root;
    for a in args
        .iter()
        .map(AsRef::as_ref)
        .filter(|a| !a.starts_with('-'))
    {
        match cur.find_subcommand(a) {
            Some(sub) => {
                path.push(' ');
                path.push_str(sub.get_name());
                cur = sub;
            }
            None => break,
        }
    }
    path
}

/// The first paragraph of a clap error on one line, without the "error: "
/// prefix (e.g. "the following required arguments were not provided:
/// --cpf-cnpj <cpf-cnpj>"); usage and tips are dropped.
fn clap_message(e: &clap::Error) -> String {
    let rendered = e.render().to_string();
    let msg = rendered
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    let msg = msg.trim_start_matches("error:").trim();
    if msg.is_empty() {
        "invalid arguments".to_string()
    } else {
        msg.to_string()
    }
}
