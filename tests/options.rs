use ollama_proxy::options::merge;
use serde_json::{json, Map, Value};

fn defaults(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn body(value: Value) -> Vec<u8> {
    serde_json::to_vec(&value).unwrap()
}

/// Most cases exercise one side of the merge, so the other is empty.
fn none() -> Map<String, Value> {
    Map::new()
}

#[test]
fn adds_options_object_when_request_has_none() {
    let raw = body(json!({ "model": "gemma4:12b", "prompt": "hi" }));
    let merged = merge(&raw, &none(), &defaults(&[("num_ctx", json!(8192))])).expect("should change");

    let out: Value = serde_json::from_slice(&merged.body).unwrap();
    assert_eq!(out["options"]["num_ctx"], 8192);
    assert_eq!(out["prompt"], "hi", "unrelated fields survive");
    assert_eq!(merged.applied, ["num_ctx"]);
}

#[test]
fn client_value_always_wins() {
    let raw = body(json!({ "model": "gemma4:12b", "options": { "num_ctx": 32768 } }));
    assert!(
        merge(&raw, &none(), &defaults(&[("num_ctx", json!(8192))])).is_none(),
        "nothing to add, so the body is forwarded untouched"
    );
}

#[test]
fn fills_only_the_missing_keys() {
    let raw = body(json!({ "model": "gemma4:12b", "options": { "temperature": 0.5 } }));
    let merged = merge(
        &raw,
        &none(),
        &defaults(&[("num_ctx", json!(8192)), ("temperature", json!(0.9))]),
    )
    .expect("should change");

    let out: Value = serde_json::from_slice(&merged.body).unwrap();
    assert_eq!(out["options"]["num_ctx"], 8192);
    assert_eq!(out["options"]["temperature"], 0.5);
    assert_eq!(merged.applied, ["num_ctx"]);
}

#[test]
fn empty_defaults_leave_the_body_alone() {
    let raw = body(json!({ "model": "gemma4:12b" }));
    assert!(merge(&raw, &none(), &none()).is_none());
}

#[test]
fn non_json_body_is_left_alone() {
    assert!(merge(b"not json at all", &none(), &defaults(&[("num_ctx", json!(8192))])).is_none());
}

#[test]
fn non_object_options_are_not_clobbered() {
    let raw = body(json!({ "model": "gemma4:12b", "options": "surprise" }));
    assert!(merge(&raw, &none(), &defaults(&[("num_ctx", json!(8192))])).is_none());
}

#[test]
fn adds_a_top_level_field_the_client_omitted() {
    let raw = body(json!({ "model": "qwen3.6:35b-a3b", "prompt": "hi" }));
    let merged = merge(&raw, &defaults(&[("think", json!(false))]), &none()).expect("should change");

    let out: Value = serde_json::from_slice(&merged.body).unwrap();
    assert_eq!(out["think"], false);
    assert_eq!(out["prompt"], "hi");
    assert_eq!(merged.applied, ["think"]);
}

#[test]
fn an_explicit_think_survives_the_default() {
    let raw = body(json!({ "model": "qwen3.6:35b-a3b", "think": true }));
    assert!(
        merge(&raw, &defaults(&[("think", json!(false))]), &none()).is_none(),
        "a client asking to think is never overridden"
    );
}

#[test]
fn think_false_is_still_left_alone() {
    let raw = body(json!({ "model": "qwen3.6:35b-a3b", "think": false }));
    assert!(
        merge(&raw, &defaults(&[("think", json!(true))]), &none()).is_none(),
        "presence is what counts, not the value"
    );
}

#[test]
fn fills_both_sides_in_one_pass() {
    let raw = body(json!({ "model": "qwen3.6:35b-a3b" }));
    let merged = merge(
        &raw,
        &defaults(&[("think", json!(false))]),
        &defaults(&[("num_ctx", json!(16384))]),
    )
    .expect("should change");

    let out: Value = serde_json::from_slice(&merged.body).unwrap();
    assert_eq!(out["think"], false);
    assert_eq!(out["options"]["num_ctx"], 16384);
    assert_eq!(merged.applied.len(), 2);
}

#[test]
fn top_level_fields_survive_a_non_object_options() {
    let raw = body(json!({ "model": "qwen3.6:35b-a3b", "options": "surprise" }));
    let merged = merge(&raw, &defaults(&[("think", json!(false))]), &none()).expect("should change");

    let out: Value = serde_json::from_slice(&merged.body).unwrap();
    assert_eq!(out["think"], false);
    assert_eq!(out["options"], "surprise", "left exactly as the client sent it");
}
