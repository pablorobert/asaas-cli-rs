use crate::client::Environment;

use super::{ConfigError, Env, Store, normalize_account_name};

// Credential sources.
pub const SOURCE_ENV: &str = "environment_variable";
pub const SOURCE_FILE: &str = "config_file";

// Environment sources, i.e. what decided sandbox vs production.
pub const ENV_FROM_FLAG: &str = "flag";
pub const ENV_FROM_ACCOUNT: &str = "account";
pub const ENV_FROM_VARIABLE: &str = "ASAAS_ENV";
pub const ENV_FROM_KEY_PREFIX: &str = "api_key_prefix";
pub const ENV_FROM_DEFAULT: &str = "default";

/// The per-invocation global flags.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    pub sandbox: bool,
    pub production: bool,
    pub account: String,
}

impl Overrides {
    /// The environment forced by --sandbox/--production.
    pub fn flag_environment(&self) -> Option<Environment> {
        if self.sandbox {
            Some(Environment::Sandbox)
        } else if self.production {
            Some(Environment::Production)
        } else {
            None
        }
    }
}

/// The fully resolved authentication context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credentials {
    pub source: &'static str,
    pub account: String,
    pub api_key: String,
    pub environment: Environment,
    pub environment_source: &'static str,
    pub base_url: String,
    pub config_file: String,
}

/// Applies the authentication hierarchy:
///
///  1. --account <name>: that account from the config file (explicit flag wins).
///  2. ASAAS_API_KEY: used as-is, the config file is ignored.
///  3. active_account from the config file.
///
/// The environment is then taken from --sandbox/--production when given,
/// otherwise from the account (file) or ASAAS_ENV / key prefix / production (env).
pub fn resolve(env: &Env, store: &Store, o: &Overrides) -> Result<Credentials, ConfigError> {
    let mut creds = if !o.account.is_empty() {
        from_file(store, &o.account)?
    } else if !env.api_key.is_empty() {
        from_env(env)?
    } else {
        from_file(store, "")?
    };

    if let Some(e) = o.flag_environment() {
        creds.environment = e;
        creds.environment_source = ENV_FROM_FLAG;
    }
    creds.base_url = if env.base_url.is_empty() {
        creds.environment.base_url().to_string()
    } else {
        env.base_url.clone()
    };
    Ok(creds)
}

fn from_env(env: &Env) -> Result<Credentials, ConfigError> {
    let (environment, environment_source) = if !env.environment.is_empty() {
        let e = Environment::parse(&env.environment).map_err(|err| ConfigError::EnvVar {
            name: "ASAAS_ENV".into(),
            err,
        })?;
        (e, ENV_FROM_VARIABLE)
    } else if let Some(e) = Environment::from_key(&env.api_key) {
        (e, ENV_FROM_KEY_PREFIX)
    } else {
        (Environment::Production, ENV_FROM_DEFAULT)
    };
    Ok(Credentials {
        source: SOURCE_ENV,
        account: String::new(),
        api_key: env.api_key.clone(),
        environment,
        environment_source,
        base_url: String::new(),
        config_file: String::new(),
    })
}

fn from_file(store: &Store, name: &str) -> Result<Credentials, ConfigError> {
    let cfg = store.load()?;
    let name = if name.is_empty() {
        if cfg.active_account.is_empty() {
            return Err(ConfigError::NoCredentials);
        }
        cfg.active_account.as_str()
    } else {
        name
    };
    let name = normalize_account_name(name)?;
    let Some(acc) = cfg.accounts.get(&name) else {
        return Err(ConfigError::AccountNotFound {
            name,
            available: cfg.account_names(),
        });
    };
    if acc.api_key.is_empty() {
        return Err(ConfigError::InvalidAccount {
            name,
            reason: "api_key is empty".into(),
        });
    }
    let environment =
        Environment::parse(&acc.environment).map_err(|reason| ConfigError::InvalidAccount {
            name: name.clone(),
            reason,
        })?;
    Ok(Credentials {
        source: SOURCE_FILE,
        api_key: acc.api_key.clone(),
        account: name,
        environment,
        environment_source: ENV_FROM_ACCOUNT,
        base_url: String::new(),
        config_file: store.path().display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Account;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("config.json"));
        (dir, store)
    }

    #[test]
    fn resolve_priority() {
        let (_tmp, s) = temp_store();
        let mut cfg = s.load().unwrap();
        cfg.active_account = "prod".into();
        cfg.accounts.insert(
            "prod".into(),
            Account {
                api_key: "file-prod-key".into(),
                environment: "production".into(),
            },
        );
        cfg.accounts.insert(
            "sbx".into(),
            Account {
                api_key: "file-sbx-key".into(),
                environment: "sandbox".into(),
            },
        );
        s.save(&cfg).unwrap();

        let env = |key: &str, e: &str| Env {
            api_key: key.into(),
            environment: e.into(),
            ..Env::default()
        };
        let sandbox = Overrides {
            sandbox: true,
            ..Overrides::default()
        };
        let account = |a: &str| Overrides {
            account: a.into(),
            ..Overrides::default()
        };
        use Environment::{Production, Sandbox};
        let cases = [
            (
                "active account",
                env("", ""),
                Overrides::default(),
                "file-prod-key",
                Production,
                SOURCE_FILE,
                ENV_FROM_ACCOUNT,
            ),
            (
                "flag overrides account env",
                env("", ""),
                sandbox.clone(),
                "file-prod-key",
                Sandbox,
                SOURCE_FILE,
                ENV_FROM_FLAG,
            ),
            (
                "env var beats file",
                env("env-key", ""),
                Overrides::default(),
                "env-key",
                Production,
                SOURCE_ENV,
                ENV_FROM_DEFAULT,
            ),
            (
                "env var with --sandbox",
                env("env-key", ""),
                sandbox,
                "env-key",
                Sandbox,
                SOURCE_ENV,
                ENV_FROM_FLAG,
            ),
            (
                "env var with ASAAS_ENV",
                env("env-key", "sandbox"),
                Overrides::default(),
                "env-key",
                Sandbox,
                SOURCE_ENV,
                ENV_FROM_VARIABLE,
            ),
            (
                "env var key prefix",
                env("$aact_hmlg_x", ""),
                Overrides::default(),
                "$aact_hmlg_x",
                Sandbox,
                SOURCE_ENV,
                ENV_FROM_KEY_PREFIX,
            ),
            (
                "--account beats env var",
                env("env-key", ""),
                account("SBX"),
                "file-sbx-key",
                Sandbox,
                SOURCE_FILE,
                ENV_FROM_ACCOUNT,
            ),
        ];
        for (name, e, o, key, environment, source, env_src) in cases {
            let c = resolve(&e, &s, &o).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(
                (
                    c.api_key.as_str(),
                    c.environment,
                    c.source,
                    c.environment_source
                ),
                (key, environment, source, env_src),
                "{name}"
            );
            assert_eq!(c.base_url, c.environment.base_url(), "{name}: base url");
        }
    }

    #[test]
    fn resolve_errors() {
        let (_tmp, s) = temp_store();
        assert_eq!(
            resolve(&Env::default(), &s, &Overrides::default()),
            Err(ConfigError::NoCredentials)
        );
        let ghost = Overrides {
            account: "ghost".into(),
            ..Overrides::default()
        };
        assert!(matches!(
            resolve(&Env::default(), &s, &ghost),
            Err(ConfigError::AccountNotFound { .. })
        ));
        let bad_env = Env {
            api_key: "k".into(),
            environment: "staging".into(),
            ..Env::default()
        };
        assert!(matches!(
            resolve(&bad_env, &s, &Overrides::default()),
            Err(ConfigError::EnvVar { .. })
        ));
        let base = Env {
            api_key: "k".into(),
            base_url: "http://localhost:9/v3".into(),
            ..Env::default()
        };
        assert_eq!(
            resolve(&base, &s, &Overrides::default()).unwrap().base_url,
            "http://localhost:9/v3"
        );
    }
}
