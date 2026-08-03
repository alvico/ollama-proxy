use ollama_proxy::routing::resolve;
use std::collections::HashMap;

fn routes(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn unmatched_model_falls_back_to_default() {
    let routes = routes(&[("kimi-linear", "f16")]);
    assert_eq!(resolve(&routes, "q8", "gemma4:12b"), "q8");
}

#[test]
fn empty_routes_always_use_default() {
    assert_eq!(resolve(&HashMap::new(), "q8", "anything"), "q8");
}

#[test]
fn prefix_matches_every_tag() {
    let routes = routes(&[("kimi-linear", "f16")]);
    assert_eq!(resolve(&routes, "q8", "kimi-linear:latest"), "f16");
    assert_eq!(resolve(&routes, "q8", "kimi-linear:q4"), "f16");
}

#[test]
fn longest_prefix_wins() {
    let routes = routes(&[("qwen3", "q8"), ("qwen3-coder", "coder")]);
    assert_eq!(resolve(&routes, "default", "qwen3-coder:30b"), "coder");
    assert_eq!(resolve(&routes, "default", "qwen3.5:9b"), "q8");
}

#[test]
fn exact_name_matches_itself() {
    let routes = routes(&[("gemma4:12b", "q8")]);
    assert_eq!(resolve(&routes, "f16", "gemma4:12b"), "q8");
}

#[test]
fn partial_prefix_does_not_match_unrelated_model() {
    let routes = routes(&[("kimi-linear", "f16")]);
    assert_eq!(resolve(&routes, "q8", "kimi"), "q8");
}

/// How `defaults` applies think:false to every model without listing them.
#[test]
fn the_empty_prefix_matches_everything_and_loses_to_any_other() {
    let routes = routes(&[("", "all"), ("kimi-linear", "f16")]);
    assert_eq!(resolve(&routes, "unused", "gemma4:12b"), "all");
    assert_eq!(resolve(&routes, "unused", "kimi-linear:latest"), "f16");
}
