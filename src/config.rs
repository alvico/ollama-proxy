use serde::Deserialize;
use serde_json::{Map, Value};
use std::{collections::HashMap, fmt, path::Path};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub listen: String,
    pub backends: HashMap<String, String>,
    pub default_backend: String,
    #[serde(default)]
    pub routes: HashMap<String, String>,
    /// Request options filled in per model when the client omits them, matched
    /// by the same longest prefix as `routes`.
    #[serde(default)]
    pub options: HashMap<String, Map<String, Value>>,
    /// The same, for fields that sit beside `options` rather than inside it.
    /// `think` is one: it is a top-level field of the ollama request, so it
    /// cannot be defaulted through `options`.
    #[serde(default)]
    pub defaults: HashMap<String, Map<String, Value>>,
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Parse(serde_json::Error),
    NoBackends,
    UnknownBackend { referenced_by: String, name: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot read config: {e}"),
            Self::Parse(e) => write!(f, "invalid config: {e}"),
            Self::NoBackends => write!(f, "no backends defined"),
            Self::UnknownBackend {
                referenced_by,
                name,
            } => write!(f, "{referenced_by} refers to undefined backend '{name}'"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
        raw.parse()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.backends.is_empty() {
            return Err(ConfigError::NoBackends);
        }
        if !self.backends.contains_key(&self.default_backend) {
            return Err(ConfigError::UnknownBackend {
                referenced_by: "default_backend".into(),
                name: self.default_backend.clone(),
            });
        }
        for (prefix, backend) in &self.routes {
            if !self.backends.contains_key(backend) {
                return Err(ConfigError::UnknownBackend {
                    referenced_by: format!("route '{prefix}'"),
                    name: backend.clone(),
                });
            }
        }
        Ok(())
    }
}

impl std::str::FromStr for Config {
    type Err = ConfigError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let cfg: Config = serde_json::from_str(raw).map_err(ConfigError::Parse)?;
        cfg.validate()?;
        Ok(cfg)
    }
}
