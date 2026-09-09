use ollama_proxy::config::parse_listen_override;
use ollama_proxy::Config;

// Addresses here come from RFC 5737's documentation range, which is reserved
// for exactly this and belongs to no real host.
const EXAMPLE: &str = "192.0.2.10:11434";

/// The smallest config that validates, with `listen` spliced in as raw JSON so
/// both the string and the list form go through the real parser.
fn with_listen(listen: &str) -> Result<Config, ollama_proxy::ConfigError> {
    format!(
        r#"{{
            "listen": {listen},
            "backends": {{ "f16": "http://127.0.0.1:11436" }},
            "default_backend": "f16"
        }}"#
    )
    .parse()
}

#[test]
fn a_bare_string_is_still_accepted() {
    let config = with_listen(r#""0.0.0.0:11434""#).expect("the pre-list form must keep working");
    assert_eq!(config.listen.addresses(), ["0.0.0.0:11434"]);
}

#[test]
fn a_list_keeps_every_address_in_order() {
    let config = with_listen(&format!(r#"["127.0.0.1:11434", "{EXAMPLE}"]"#)).expect("should parse");
    assert_eq!(config.listen.addresses(), ["127.0.0.1:11434", EXAMPLE]);
}

#[test]
fn a_single_element_list_is_the_same_as_the_string() {
    let list = with_listen(r#"["127.0.0.1:11434"]"#).expect("should parse");
    let string = with_listen(r#""127.0.0.1:11434""#).expect("should parse");
    assert_eq!(list.listen.addresses(), string.listen.addresses());
}

#[test]
fn an_empty_list_is_rejected() {
    // Nothing to bind is a config mistake, not a proxy that listens nowhere.
    assert!(
        matches!(
            with_listen("[]"),
            Err(ollama_proxy::ConfigError::NoListenAddress)
        ),
        "an empty listen list must not validate"
    );
}

#[test]
fn the_override_splits_on_commas_and_trims() {
    assert_eq!(
        parse_listen_override(&format!(" 127.0.0.1:11434 , {EXAMPLE} ")),
        ["127.0.0.1:11434", EXAMPLE]
    );
}

#[test]
fn the_override_accepts_a_single_address() {
    assert_eq!(parse_listen_override(EXAMPLE), [EXAMPLE]);
}

#[test]
fn a_blank_override_yields_nothing_so_the_file_still_wins() {
    // An exported-but-empty variable is a common shell accident, and it must
    // not leave the proxy with no address at all.
    assert!(parse_listen_override("   ").is_empty());
    assert!(parse_listen_override(",,").is_empty());
}
