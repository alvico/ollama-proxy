use ollama_proxy::{Config, ConfigError};

const VALID: &str = r#"{
    "listen": "0.0.0.0:11434",
    "backends": { "q8": "http://127.0.0.1:11435", "f16": "http://127.0.0.1:11436" },
    "default_backend": "q8",
    "routes": { "kimi-linear": "f16" }
}"#;

#[test]
fn parses_a_valid_config() {
    let config: Config = VALID.parse().expect("should parse");
    assert_eq!(config.listen, "0.0.0.0:11434");
    assert_eq!(config.backends.len(), 2);
    assert_eq!(config.routes["kimi-linear"], "f16");
}

#[test]
fn routes_may_be_omitted() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "q8": "http://127.0.0.1:11435" },
        "default_backend": "q8"
    }"#;
    let config: Config = raw.parse().expect("should parse");
    assert!(config.routes.is_empty());
}

#[test]
fn unknown_keys_are_ignored() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "q8": "http://127.0.0.1:11435" },
        "default_backend": "q8",
        "_comment": "explanatory note"
    }"#;
    assert!(raw.parse::<Config>().is_ok());
}

#[test]
fn rejects_default_backend_that_does_not_exist() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "q8": "http://127.0.0.1:11435" },
        "default_backend": "nope"
    }"#;
    assert!(matches!(
        raw.parse::<Config>(),
        Err(ConfigError::UnknownBackend { .. })
    ));
}

#[test]
fn rejects_route_pointing_at_undefined_backend() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "q8": "http://127.0.0.1:11435" },
        "default_backend": "q8",
        "routes": { "kimi-linear": "f16" }
    }"#;
    assert!(matches!(
        raw.parse::<Config>(),
        Err(ConfigError::UnknownBackend { .. })
    ));
}

#[test]
fn rejects_empty_backends() {
    let raw = r#"{ "listen": "127.0.0.1:11434", "backends": {}, "default_backend": "q8" }"#;
    assert!(matches!(raw.parse::<Config>(), Err(ConfigError::NoBackends)));
}

#[test]
fn rejects_malformed_json() {
    assert!(matches!("{ not json".parse::<Config>(), Err(ConfigError::Parse(_))));
}

#[test]
fn defaults_may_be_omitted() {
    let config: Config = VALID.parse().expect("should parse");
    assert!(config.defaults.is_empty());
}

#[test]
fn parses_top_level_field_defaults() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "q8": "http://127.0.0.1:11435" },
        "default_backend": "q8",
        "defaults": { "": { "think": false } }
    }"#;
    let config: Config = raw.parse().expect("should parse");
    assert_eq!(config.defaults[""]["think"], false);
}
