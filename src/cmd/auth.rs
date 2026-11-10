//! `asaas auth`: local multi-account management.

use clap::{ArgMatches, Command};
use serde_json::{Value, json};

use crate::client::{Environment, Query, mask_key};
use crate::cmdutil::{
    self, Error, Factory, Kind, Result, bool_arg, get_bool, get_string, mark_required, object,
    string_arg,
};
use crate::config::{self, Account, ConfigError, Overrides, SOURCE_FILE};
use crate::usage_err;

pub fn command() -> Command {
    cmdutil::new_group(
        "auth",
        "asaas auth <subcommand>",
        "Manage Asaas accounts (API keys) stored in the local config file",
        r#"Manage the accounts stored in ~/.config/asaas/config.json
(or $ASAAS_CONFIG_DIR/config.json). The file maps account nicknames to an
API key and an environment, plus the currently active account:

  {
    "active_account": "sandbox-dev",
    "accounts": {
      "sandbox-dev":    {"api_key": "$aact_hmlg_...", "environment": "sandbox"},
      "producao-firma": {"api_key": "$aact_prod_...", "environment": "production"}
    }
  }

The file is written with 0600 permissions. When ASAAS_API_KEY is set, API
commands use it and ignore this file; 'auth add/switch/list/remove' still
edit the file."#,
    )
    .subcommands([
        cmd_add(),
        cmd_switch(),
        cmd_status(),
        cmd_list(),
        cmd_remove(),
    ])
}

pub fn run(f: &mut Factory, cmd: &mut Command, m: &ArgMatches) -> Result<()> {
    match m.subcommand() {
        Some(("add", sub)) => run_add(f, sub),
        Some(("switch", sub)) => run_switch(f, sub),
        Some(("status", sub)) => run_status(f, sub),
        Some(("list", _)) => run_list(f),
        Some(("remove", sub)) => run_remove(f, sub),
        _ => cmdutil::print_group_help(f, cmd),
    }
}

/// The JSON shape used to describe an account.
fn masked_view(name: &str, key: &str, env: &str, active: bool) -> Value {
    json!({"name": name, "environment": env, "api_key": mask_key(key), "active": active})
}

fn cmd_add() -> Command {
    let cmd = cmdutil::new_command(
        "add",
        "asaas auth add --name <name> --key <api-key> [--sandbox | --production]",
        "Add an Asaas account (nickname + API key + environment) to the config file",
        r#"Add an account to ~/.config/asaas/config.json and make it the active account.

The environment is taken from the global --sandbox or --production flag. When
neither is given it is inferred from the key prefix ($aact_hmlg_ = sandbox,
$aact_prod_ = production); if the key has no known prefix the command fails
and asks for an explicit flag. A flag that contradicts the key prefix is an
error.

Use --key - to read the key from stdin, keeping it out of shell history and
the process list. The key is not validated against the API; run
'asaas auth status --verify' afterwards to check it.

Output (stdout):
  {"account": {"name", "environment", "api_key" (masked), "active"},
   "config_file": string, "previous_active_account": string, "replaced": bool}"#,
    );
    cmdutil::with_examples(
        cmd,
        r#"  asaas auth add --name sandbox-dev --key "$ASAAS_SANDBOX_KEY" --sandbox
  printf '%s' "$KEY" | asaas auth add --name producao-firma --key - --production
  asaas auth add --name sandbox-dev --key "$NEW_KEY" --sandbox --force"#,
    )
    .arg(mark_required(string_arg("name", "Account nickname, 1-64 chars of [a-z0-9_-], case-insensitive (e.g. sandbox-dev) [type: string]")))
    .arg(mark_required(string_arg("key", "Asaas API key, or '-' to read it from stdin [type: string]")))
    .arg(bool_arg("force", "Overwrite the account if the nickname already exists [type: boolean]"))
    .arg(bool_arg("activate", "Make this the active account; use --activate=false to keep the current one [type: boolean]").default_value("true"))
}

fn run_add(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let name = config::normalize_account_name(&get_string(m, "name"))
        .map_err(|e| usage_err!("--name: {e}"))?;

    let mut key = get_string(m, "key");
    if key == "-" {
        key = f
            .read_line()
            .map_err(|e| usage_err!("--key -: could not read the API key from stdin: {e}"))?;
    }
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err(usage_err!("--key: API key is empty"));
    }

    let env = account_environment(&f.overrides(), &key)?;

    let store = f.store()?;
    let mut cfg = store.load()?;

    let exists = cfg.accounts.contains_key(&name);
    if exists && !get_bool(m, "force") {
        return Err(Error::Cli {
            kind: Kind::Config,
            message: format!("account {name} already exists"),
            hint: Some("pass --force to overwrite it".into()),
        });
    }

    let previous = cfg.active_account.clone();
    cfg.accounts.insert(
        name.clone(),
        Account {
            api_key: key.clone(),
            environment: env.to_string(),
        },
    );
    let previous_valid = cfg.accounts.contains_key(&previous);
    if get_bool(m, "activate") || previous.is_empty() || !previous_valid {
        cfg.active_account = name.clone();
    }
    store.save(&cfg)?;

    let mut pairs = vec![
        (
            "account",
            masked_view(&name, &key, env.as_str(), cfg.active_account == name),
        ),
        ("replaced", json!(exists)),
        ("previous_active_account", json!(previous)),
        ("config_file", json!(store.path().display().to_string())),
    ];
    if !f.env().api_key.is_empty() {
        pairs.push((
            "warning",
            json!(
                "ASAAS_API_KEY is set and takes precedence over the config file for API commands"
            ),
        ));
    }
    f.print(&object(pairs))
}

/// Picks the environment for a new account from the global flags or the key
/// prefix, rejecting contradictions.
fn account_environment(o: &Overrides, key: &str) -> Result<Environment> {
    match (o.flag_environment(), Environment::from_key(key)) {
        (Some(flag), Some(prefix)) if flag != prefix => Err(usage_err!(
            "the API key prefix indicates a {prefix} key, but --{flag} was given"
        )),
        (Some(flag), _) => Ok(flag),
        (None, Some(prefix)) => Ok(prefix),
        (None, None) => Err(usage_err!(
            "cannot infer the environment from the API key; pass --sandbox or --production"
        )),
    }
}

fn cmd_switch() -> Command {
    let cmd = cmdutil::new_command(
        "switch",
        "asaas auth switch --name <name>",
        "Change the active account in the config file",
        r#"Set "active_account" in ~/.config/asaas/config.json to an existing account.
Subsequent API commands use it unless ASAAS_API_KEY or --account is given.

Output (stdout):
  {"account": {"name", "environment", "api_key" (masked), "active": true},
   "config_file": string, "previous_active_account": string}

Fails with config_error (exit 3) and "available_accounts" when the name is unknown."#,
    );
    cmdutil::with_examples(cmd, "  asaas auth switch --name producao-firma").arg(mark_required(
        string_arg(
            "name",
            "Nickname of an existing account to activate [type: string]",
        ),
    ))
}

fn run_switch(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let name = config::normalize_account_name(&get_string(m, "name"))
        .map_err(|e| usage_err!("--name: {e}"))?;
    let store = f.store()?;
    let mut cfg = store.load()?;
    let Some(acc) = cfg.accounts.get(&name).cloned() else {
        return Err(ConfigError::AccountNotFound {
            name,
            available: cfg.account_names(),
        }
        .into());
    };
    let previous = std::mem::replace(&mut cfg.active_account, name.clone());
    store.save(&cfg)?;

    let mut pairs = vec![
        (
            "account",
            masked_view(&name, &acc.api_key, &acc.environment, true),
        ),
        ("previous_active_account", json!(previous)),
        ("config_file", json!(store.path().display().to_string())),
    ];
    if !f.env().api_key.is_empty() {
        pairs.push((
            "warning",
            json!(
                "ASAAS_API_KEY is set and takes precedence over the active account for API commands"
            ),
        ));
    }
    f.print(&object(pairs))
}

fn cmd_status() -> Command {
    let cmd = cmdutil::new_command(
        "status",
        "asaas auth status",
        "Show which account, key and environment API commands will use",
        r#"Resolve the authentication context exactly as API commands do (honoring
ASAAS_API_KEY, --account, --sandbox and --production) and print it. The API
key is masked: only its environment prefix and last 4 characters are shown.

Output (stdout):
  {
    "account": string (config_file only),
    "api_key": string (masked),
    "base_url": string,
    "config_file": string,
    "environment": "sandbox" | "production",
    "environment_source": "flag" | "account" | "ASAAS_ENV" | "api_key_prefix" | "default",
    "source": "environment_variable" | "config_file",
    "verified": bool (only with --verify)
  }

With --verify, a real request (GET /customers?limit=1) checks that the key is
accepted; a rejected key returns api_error (exit 1) on stderr. Without
credentials, fails with auth_error (exit 3)."#,
    );
    cmdutil::with_examples(
        cmd,
        r#"  asaas auth status
  asaas auth status --verify
  ASAAS_API_KEY="$KEY" asaas auth status --sandbox"#,
    )
    .arg(bool_arg(
        "verify",
        "Also send a real API request to confirm the key is accepted [type: boolean]",
    ))
}

fn run_status(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let creds = f.credentials()?;
    let mut pairs = vec![
        ("source", json!(creds.source)),
        ("environment", json!(creds.environment.as_str())),
        ("environment_source", json!(creds.environment_source)),
        ("base_url", json!(creds.base_url)),
        ("api_key", json!(mask_key(&creds.api_key))),
    ];
    if creds.source == SOURCE_FILE {
        pairs.push(("account", json!(creds.account)));
        pairs.push(("config_file", json!(creds.config_file)));
    } else if let Ok(store) = f.store() {
        pairs.push(("config_file", json!(store.path().display().to_string())));
        pairs.push((
            "note",
            json!("ASAAS_API_KEY is set; the config file is ignored"),
        ));
    }

    if get_bool(m, "verify") {
        let client = f.client()?;
        let mut q = Query::new();
        q.set("limit", "1");
        client.get("/customers", Some(&q))?;
        pairs.push(("verified", json!(true)));
    }
    f.print(&object(pairs))
}

fn cmd_list() -> Command {
    cmdutil::new_command(
        "list",
        "asaas auth list",
        "List the accounts stored in the config file (keys masked)",
        r#"List every account in ~/.config/asaas/config.json, sorted by name.

Output (stdout):
  {"accounts": [{"name", "environment", "api_key" (masked), "active"}],
   "active_account": string, "config_file": string}"#,
    )
}

fn run_list(f: &mut Factory) -> Result<()> {
    let store = f.store()?;
    let cfg = store.load()?;
    let accounts: Vec<Value> = cfg
        .accounts
        .iter()
        .map(|(n, acc)| masked_view(n, &acc.api_key, &acc.environment, *n == cfg.active_account))
        .collect();
    f.print(&object([
        ("active_account", json!(cfg.active_account)),
        ("config_file", json!(store.path().display().to_string())),
        ("accounts", Value::Array(accounts)),
    ]))
}

fn cmd_remove() -> Command {
    let cmd = cmdutil::new_command(
        "remove",
        "asaas auth remove --name <name>",
        "Remove an account from the config file",
        r#"Delete an account from ~/.config/asaas/config.json. Removing the active
account leaves "active_account" empty; pick another with 'asaas auth switch'.

Output (stdout):
  {"active_account": string, "config_file": string, "removed": string}"#,
    );
    cmdutil::with_examples(cmd, "  asaas auth remove --name sandbox-dev").arg(mark_required(
        string_arg("name", "Nickname of the account to remove [type: string]"),
    ))
}

fn run_remove(f: &mut Factory, m: &ArgMatches) -> Result<()> {
    let name = config::normalize_account_name(&get_string(m, "name"))
        .map_err(|e| usage_err!("--name: {e}"))?;
    let store = f.store()?;
    let mut cfg = store.load()?;
    if cfg.accounts.remove(&name).is_none() {
        return Err(ConfigError::AccountNotFound {
            name,
            available: cfg.account_names(),
        }
        .into());
    }
    if cfg.active_account == name {
        cfg.active_account.clear();
    }
    store.save(&cfg)?;
    f.print(&object([
        ("removed", json!(name)),
        ("active_account", json!(cfg.active_account)),
        ("config_file", json!(store.path().display().to_string())),
    ]))
}
