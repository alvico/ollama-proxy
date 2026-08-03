use ollama_proxy::backend::merge_models;
use serde_json::json;

#[test]
fn merges_and_deduplicates_overlapping_listings() {
    let a = json!({ "models": [{ "name": "gemma4:12b" }, { "name": "qwen3.5:9b" }] });
    let b = json!({ "models": [{ "name": "qwen3.5:9b" }, { "name": "kimi-linear:latest" }] });

    let merged = merge_models(&[a, b]);
    let names: Vec<&str> = merged["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["name"].as_str().unwrap())
        .collect();

    assert_eq!(names, ["gemma4:12b", "qwen3.5:9b", "kimi-linear:latest"]);
}

#[test]
fn tolerates_backends_returning_nothing() {
    let merged = merge_models(&[json!({}), json!({ "models": [{ "name": "gemma4:12b" }] })]);
    assert_eq!(merged["models"].as_array().unwrap().len(), 1);
}

#[test]
fn empty_input_yields_empty_list() {
    let merged = merge_models(&[]);
    assert!(merged["models"].as_array().unwrap().is_empty());
}
