use std::fmt;

/// Identifies which Asaas deployment a request targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Sandbox,
}

/// Base URL of the production API v3.
pub const PRODUCTION_BASE_URL: &str = "https://api.asaas.com/v3";
/// Base URL of the sandbox API v3.
pub const SANDBOX_BASE_URL: &str = "https://api-sandbox.asaas.com/v3";

// API key prefixes issued by Asaas. Keys generated in the sandbox
// ("homologação") start with $aact_hmlg_, production keys with $aact_prod_.
const PRODUCTION_KEY_PREFIX: &str = "$aact_prod_";
const SANDBOX_KEY_PREFIX: &str = "$aact_hmlg_";

impl Environment {
    /// The name used in the config file and in JSON output.
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Production => "production",
            Environment::Sandbox => "sandbox",
        }
    }

    /// The API v3 base URL for the environment.
    pub fn base_url(self) -> &'static str {
        match self {
            Environment::Production => PRODUCTION_BASE_URL,
            Environment::Sandbox => SANDBOX_BASE_URL,
        }
    }

    /// Converts user input ("sandbox", " Production ") into an Environment.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "production" => Ok(Environment::Production),
            "sandbox" => Ok(Environment::Sandbox),
            _ => Err(format!(
                "invalid environment {s:?}: must be \"production\" or \"sandbox\""
            )),
        }
    }

    /// Infers the environment from the API key prefix; None when the key has
    /// no recognizable prefix.
    pub fn from_key(key: &str) -> Option<Self> {
        if key.starts_with(PRODUCTION_KEY_PREFIX) {
            Some(Environment::Production)
        } else if key.starts_with(SANDBOX_KEY_PREFIX) {
            Some(Environment::Sandbox)
        } else {
            None
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Hides the secret part of an API key, keeping only the non-secret
/// environment prefix (when present) and the last 4 characters.
pub fn mask_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let (mut prefix, rest) = [PRODUCTION_KEY_PREFIX, SANDBOX_KEY_PREFIX]
        .iter()
        .find_map(|p| key.strip_prefix(p).map(|r| (p.to_string(), r)))
        .unwrap_or((String::new(), key));
    let chars: Vec<char> = rest.chars().collect();
    if chars.len() < 12 {
        return prefix + "****";
    }
    if prefix.is_empty() {
        prefix = chars[..4].iter().collect();
    }
    let tail: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}****{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_helpers() {
        assert_eq!(
            Environment::from_key("$aact_hmlg_abc"),
            Some(Environment::Sandbox)
        );
        assert_eq!(
            Environment::from_key("$aact_prod_abc"),
            Some(Environment::Production)
        );
        assert_eq!(Environment::from_key("whatever"), None);
        assert_eq!(Environment::parse(" Sandbox "), Ok(Environment::Sandbox));
        assert!(Environment::parse("staging").is_err());

        let cases = [
            (
                "$aact_hmlg_000MzkwODA2MWY2OGM3MWRlMDU2NWM3MzJl",
                "$aact_hmlg_****MzJl",
            ),
            ("$aact_prod_short", "$aact_prod_****"),
            ("plainkey1234567890abcd", "plai****abcd"),
            ("tiny", "****"),
            ("", ""),
        ];
        for (input, want) in cases {
            assert_eq!(mask_key(input), want, "mask_key({input:?})");
        }
    }
}
