use serde::Deserialize;
use serde_json::{Map, Value};
use std::{collections::HashMap, fmt, path::Path};

/// Environment override for `listen`, comma separated. Bind addresses are a
/// property of the host rather than of the project, so this keeps a real
/// deployment's addresses out of the committed config entirely.
pub const LISTEN_ENV: &str = "OLLAMA_PROXY_LISTEN";

/// Splits the environment override. Separate from the lookup so it can be
/// tested without a test racing every other test over one process-wide
/// variable.
pub fn parse_listen_override(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// One or more bind addresses. A bare string is still accepted, so a config
/// written before this existed keeps working. A list is what lets the proxy sit
/// on the loopback and a private interface at once: the alternative, binding
/// `0.0.0.0`, also exposes every other network the host is attached to, and the
/// proxy has no authentication of its own -- anything that can reach the port
/// can drive the models and read the prompts.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Listen {
    One(String),
    Many(Vec<String>),
}

impl Listen {
    pub fn addresses(&self) -> &[String] {
        match self {
            Self::One(addr) => std::slice::from_ref(addr),
            Self::Many(addrs) => addrs,
        }
    }
}

/// What a backend speaks. Everything the proxy does beyond forwarding --
/// evicting, listing -- goes through a server-specific API, so it must know.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    #[default]
    Ollama,
    /// llama.cpp's llama-server in router mode (`--models-preset` or
    /// `--models-dir`). It serves models ollama cannot load, such as those
    /// needing a llama.cpp fork, but speaks only the OpenAI API: no `/api/*`.
    LlamaServer,
}

/// A bare URL string is an ollama backend, so configs written before kinds
/// existed keep working; other kinds use the `{ "url", "kind" }` form.
#[derive(Debug, Clone, Deserialize)]
#[serde(from = "BackendSpec")]
pub struct Backend {
    pub url: String,
    pub kind: BackendKind,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum BackendSpec {
    Url(String),
    Full {
        url: String,
        #[serde(default)]
        kind: BackendKind,
    },
}

impl From<BackendSpec> for Backend {
    fn from(spec: BackendSpec) -> Self {
        match spec {
            BackendSpec::Url(url) => Self {
                url,
                kind: BackendKind::Ollama,
            },
            BackendSpec::Full { url, kind } => Self { url, kind },
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub listen: Listen,
    pub backends: HashMap<String, Backend>,
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
    /// Groups of defaulted field names treated as one knob: if a client request
    /// already contains any member of a group, none of the group's defaults are
    /// injected. `think` and `reasoning_effort` are the same switch on the two
    /// endpoints ollama exposes, so a client pinning either must not get the
    /// other defaulted against it.
    #[serde(default)]
    pub exclusive_defaults: Vec<Vec<String>>,
}

#[derive(Debug)]
pub enum ConfigError {
    Read(std::io::Error),
    Parse(serde_json::Error),
    NoBackends,
    NoListenAddress,
    UnknownBackend { referenced_by: String, name: String },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read(e) => write!(f, "cannot read config: {e}"),
            Self::Parse(e) => write!(f, "invalid config: {e}"),
            Self::NoBackends => write!(f, "no backends defined"),
            Self::NoListenAddress => write!(f, "listen is an empty list"),
            Self::UnknownBackend {
                referenced_by,
                name,
            } => write!(f, "{referenced_by} refers to undefined backend '{name}'"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Bind addresses, with `OLLAMA_PROXY_LISTEN` taking precedence over the
    /// file so a host's own addresses need never be committed.
    pub fn listen_addresses(&self) -> Vec<String> {
        if let Ok(raw) = std::env::var(LISTEN_ENV) {
            let overridden = parse_listen_override(&raw);
            if !overridden.is_empty() {
                return overridden;
            }
            log::warn!("{LISTEN_ENV} is set but empty; using the config file");
        }
        self.listen.addresses().to_vec()
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(ConfigError::Read)?;
        raw.parse()
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.listen.addresses().is_empty() {
            return Err(ConfigError::NoListenAddress);
        }
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
