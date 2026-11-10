//! The multi-account file stored at ~/.config/asaas/config.json and the
//! ASAAS_* environment variables.

mod resolve;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use resolve::{
    Credentials, ENV_FROM_ACCOUNT, ENV_FROM_DEFAULT, ENV_FROM_FLAG, ENV_FROM_KEY_PREFIX,
    ENV_FROM_VARIABLE, Overrides, SOURCE_ENV, SOURCE_FILE, resolve,
};

const DIR_NAME: &str = "asaas";
const FILE_NAME: &str = "config.json";

/// One entry of the "accounts" map.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub environment: String,
}

/// Mirrors the JSON file:
///
/// ```json
/// {
///   "active_account": "sandbox-dev",
///   "accounts": {
///     "sandbox-dev": { "api_key": "...", "environment": "sandbox" }
///   }
/// }
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub accounts: BTreeMap<String, Account>,
    #[serde(default)]
    pub active_account: String,
}

impl Config {
    /// The configured account names, sorted.
    pub fn account_names(&self) -> Vec<String> {
        self.accounts.keys().cloned().collect()
    }
}

/// The ASAAS_* environment variables.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// ASAAS_API_KEY
    pub api_key: String,
    /// ASAAS_ENV ("sandbox" | "production")
    pub environment: String,
    /// ASAAS_BASE_URL (override, mainly for proxies/tests)
    pub base_url: String,
    /// ASAAS_CONFIG_DIR (override of the config directory)
    pub config_dir: String,
}

/// Reads the ASAAS_* variables from the process environment.
pub fn load_env() -> Env {
    let var = |name: &str| {
        std::env::var(name)
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    Env {
        api_key: var("ASAAS_API_KEY"),
        environment: var("ASAAS_ENV"),
        base_url: var("ASAAS_BASE_URL"),
        config_dir: var("ASAAS_CONFIG_DIR"),
    }
}

/// Every failure related to configuration and credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Reading or writing the config file failed.
    File {
        op: String,
        path: Option<PathBuf>,
        err: String,
    },
    /// Neither ASAAS_API_KEY nor an active account exist.
    NoCredentials,
    /// A requested account is not in the config file.
    AccountNotFound {
        name: String,
        available: Vec<String>,
    },
    /// A stored account is unusable.
    InvalidAccount { name: String, reason: String },
    /// An ASAAS_* environment variable has an invalid value.
    EnvVar { name: String, err: String },
    /// An account nickname does not follow the naming rules.
    InvalidName(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::File {
                op,
                path: None,
                err,
            } => write!(f, "config: {op}: {err}"),
            ConfigError::File {
                op,
                path: Some(p),
                err,
            } => {
                write!(f, "config: {op} {}: {err}", p.display())
            }
            ConfigError::NoCredentials => f.write_str("no Asaas credentials configured"),
            ConfigError::AccountNotFound { name, available } if available.is_empty() => {
                write!(f, "account {name:?} not found: no accounts configured")
            }
            ConfigError::AccountNotFound { name, available } => {
                write!(
                    f,
                    "account {name:?} not found (available: {})",
                    available.join(", ")
                )
            }
            ConfigError::InvalidAccount { name, reason } => {
                write!(f, "account {name:?} is invalid: {reason}")
            }
            ConfigError::EnvVar { name, err } => write!(f, "{name}: {err}"),
            ConfigError::InvalidName(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for ConfigError {}

fn file_err(op: &str, path: Option<&Path>, err: impl fmt::Display) -> ConfigError {
    ConfigError::File {
        op: op.to_string(),
        path: path.map(Path::to_path_buf),
        err: err.to_string(),
    }
}

/// The configuration directory, in order of precedence:
/// $ASAAS_CONFIG_DIR, $XDG_CONFIG_HOME/asaas, ~/.config/asaas.
pub fn dir(env: &Env) -> Result<PathBuf, ConfigError> {
    #[allow(deprecated)] // fixed on Windows since Rust 1.85
    let home = std::env::home_dir();
    dir_from(env, std::env::var_os("XDG_CONFIG_HOME"), home)
}

fn dir_from(
    env: &Env,
    xdg: Option<OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf, ConfigError> {
    if !env.config_dir.is_empty() {
        return Ok(PathBuf::from(&env.config_dir));
    }
    if let Some(x) = xdg.filter(|x| !x.is_empty()) {
        return Ok(PathBuf::from(x).join(DIR_NAME));
    }
    match home {
        Some(h) if !h.as_os_str().is_empty() => Ok(h.join(".config").join(DIR_NAME)),
        _ => Err(file_err(
            "locate home directory",
            None,
            "home directory not found",
        )),
    }
}

/// Reads and writes the config file.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
}

impl Store {
    /// A Store for an explicit file path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Store { path: path.into() }
    }

    /// The Store at dir(env)/config.json.
    pub fn default_for(env: &Env) -> Result<Self, ConfigError> {
        Ok(Store::new(dir(env)?.join(FILE_NAME)))
    }

    /// The location of the config file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads the config file. A missing or empty file yields an empty Config.
    /// Account names are lower-cased, as they are case-insensitive.
    pub fn load(&self) -> Result<Config, ConfigError> {
        let raw = match fs::read(&self.path) {
            Ok(r) => r,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(file_err("read", Some(&self.path), e)),
        };
        if raw.trim_ascii().is_empty() {
            return Ok(Config::default());
        }
        let value: Value =
            serde_json::from_slice(&raw).map_err(|e| file_err("read", Some(&self.path), e))?;
        let cfg: Config =
            serde_json::from_value(value).map_err(|e| file_err("decode", Some(&self.path), e))?;
        Ok(Config {
            accounts: cfg
                .accounts
                .into_iter()
                .map(|(k, v)| (k.to_lowercase(), v))
                .collect(),
            active_account: cfg.active_account,
        })
    }

    /// Atomically writes cfg with 0600 permissions, creating the directory
    /// (0700) when needed.
    pub fn save(&self, cfg: &Config) -> Result<(), ConfigError> {
        let dir = match self.path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            _ => PathBuf::from("."),
        };
        create_private_dir(&dir).map_err(|e| file_err("create directory", Some(&dir), e))?;

        let mut payload =
            serde_json::to_vec_pretty(cfg).map_err(|e| file_err("write", Some(&self.path), e))?;
        payload.push(b'\n');

        // Write to a temp file in the same directory, then rename, so a crash
        // never leaves a truncated config behind.
        let tmp = dir.join(format!(".config-{}.tmp.json", std::process::id()));
        let result = write_private_file(&tmp, &payload)
            .map_err(|e| file_err("write", Some(&self.path), e))
            .and_then(|()| {
                fs::rename(&tmp, &self.path).map_err(|e| file_err("write", Some(&self.path), e))
            });
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(0o600);
        let mut f = opts.open(path)?;
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.write_all(data)?;
        return f.sync_all();
    }
    #[cfg(not(unix))]
    {
        let mut f = opts.open(path)?;
        f.write_all(data)?;
        f.sync_all()
    }
}

/// Lower-cases and validates an account nickname: 1-64 characters from
/// [a-z0-9_-], starting with a letter or digit.
pub fn normalize_account_name(name: &str) -> Result<String, ConfigError> {
    let n = name.trim().to_lowercase();
    let valid = !n.is_empty()
        && n.len() <= 64
        && n.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && (b == b'_' || b == b'-'))
        });
    if !valid {
        return Err(ConfigError::InvalidName(format!(
            "invalid account name {name:?}: use 1-64 characters from [a-z0-9_-], starting with a letter or digit"
        )));
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("nested").join("asaas").join("config.json"));
        (dir, store)
    }

    #[test]
    fn store_round_trip_and_permissions() {
        let (_tmp, s) = temp_store();
        let mut cfg = s.load().unwrap();
        assert!(cfg.accounts.is_empty(), "missing file should load empty");

        cfg.active_account = "sandbox-dev".into();
        cfg.accounts.insert(
            "sandbox-dev".into(),
            Account {
                api_key: "$aact_hmlg_1".into(),
                environment: "sandbox".into(),
            },
        );
        cfg.accounts.insert(
            "producao-firma".into(),
            Account {
                api_key: "$aact_prod_2".into(),
                environment: "production".into(),
            },
        );
        s.save(&cfg).unwrap();
        // Saving over an existing file must also work (rename replaces it).
        s.save(&cfg).unwrap();

        let got = s.load().unwrap();
        assert_eq!(got.active_account, "sandbox-dev");
        assert_eq!(got.accounts["producao-firma"].api_key, "$aact_prod_2");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = fs::metadata(s.path()).unwrap().permissions().mode() & 0o777;
            assert_eq!(file, 0o600, "file perm");
            let dir = fs::metadata(s.path().parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(dir, 0o700, "dir perm");
        }
        let entries = fs::read_dir(s.path().parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1, "temp files left behind");
    }

    #[test]
    fn load_lowercases_account_names_and_tolerates_empty_file() {
        let (_tmp, s) = temp_store();
        fs::create_dir_all(s.path().parent().unwrap()).unwrap();
        fs::write(s.path(), "").unwrap();
        assert_eq!(s.load().unwrap(), Config::default());

        fs::write(s.path(), r#"{"active_account":"dev","accounts":{"Dev":{"api_key":"k","environment":"sandbox"}}}"#).unwrap();
        assert_eq!(s.load().unwrap().accounts["dev"].api_key, "k");

        fs::write(s.path(), "{not json").unwrap();
        assert!(matches!(s.load(), Err(ConfigError::File { .. })));
    }

    #[test]
    fn dir_precedence() {
        let custom = Env {
            config_dir: "/custom".into(),
            ..Env::default()
        };
        assert_eq!(
            dir_from(&custom, None, None).unwrap(),
            PathBuf::from("/custom")
        );
        let home = PathBuf::from("/home/u");
        assert_eq!(
            dir_from(&Env::default(), Some("".into()), Some(home.clone())).unwrap(),
            home.join(".config").join("asaas")
        );
        assert_eq!(
            dir_from(&Env::default(), Some("/xdg".into()), Some(home)).unwrap(),
            PathBuf::from("/xdg").join("asaas")
        );
        assert!(dir_from(&Env::default(), None, None).is_err());
    }

    #[test]
    fn normalize_account_name_rules() {
        assert_eq!(
            normalize_account_name(" Sandbox-Dev ").unwrap(),
            "sandbox-dev"
        );
        for bad in ["", "a.b", "-x", "with space", "ação", &"a".repeat(65)] {
            assert!(
                normalize_account_name(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }
}
