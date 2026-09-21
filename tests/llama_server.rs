use ollama_proxy::backend::{
    llama_as_openai, llama_as_ps, llama_as_tags, merge_models, merge_openai_models, rfc3339,
};
use ollama_proxy::{BackendKind, Config};
use serde_json::{json, Value};

/// Trimmed from a real router-mode `/models` response.
fn router_listing() -> Value {
    json!({
        "object": "list",
        "data": [
            {
                "id": "ternary-bonsai-2-27b",
                "object": "model",
                "owned_by": "llamacpp",
                "created": 1_790_012_685,
                "status": {
                    "value": "loaded",
                    "args": ["llama-server", "--port", "0", "--model", "does/not/exist.gguf"]
                }
            },
            {
                "id": "ternary-bonsai-8b",
                "object": "model",
                "owned_by": "llamacpp",
                "created": 1_790_012_685,
                "status": { "value": "unloaded", "args": [] }
            }
        ]
    })
}

fn names(listing: &Value, list: &str, key: &str) -> Vec<String> {
    listing[list]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m[key].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_bare_url_is_an_ollama_backend() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "f16": "http://127.0.0.1:11436" },
        "default_backend": "f16"
    }"#;
    let config: Config = raw.parse().unwrap();
    assert_eq!(config.backends["f16"].url, "http://127.0.0.1:11436");
    assert_eq!(config.backends["f16"].kind, BackendKind::Ollama);
}

#[test]
fn parses_a_llama_server_backend() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": {
            "f16": "http://127.0.0.1:11436",
            "bonsai": { "url": "http://127.0.0.1:11437", "kind": "llama-server" }
        },
        "default_backend": "f16",
        "routes": { "ternary-bonsai": "bonsai" }
    }"#;
    let config: Config = raw.parse().unwrap();
    assert_eq!(config.backends["bonsai"].kind, BackendKind::LlamaServer);
    assert_eq!(config.backends["bonsai"].url, "http://127.0.0.1:11437");
}

#[test]
fn object_form_defaults_to_ollama() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "f16": { "url": "http://127.0.0.1:11436" } },
        "default_backend": "f16"
    }"#;
    let config: Config = raw.parse().unwrap();
    assert_eq!(config.backends["f16"].kind, BackendKind::Ollama);
}

#[test]
fn rejects_an_unknown_kind() {
    let raw = r#"{
        "listen": "127.0.0.1:11434",
        "backends": { "x": { "url": "http://127.0.0.1:1", "kind": "vllm" } },
        "default_backend": "x"
    }"#;
    assert!(raw.parse::<Config>().is_err());
}

#[test]
fn tags_list_every_router_model() {
    let tags = llama_as_tags(&router_listing());
    assert_eq!(
        names(&tags, "models", "name"),
        ["ternary-bonsai-2-27b", "ternary-bonsai-8b"]
    );
    let first = &tags["models"][0];
    assert_eq!(first["model"], "ternary-bonsai-2-27b");
    // The GGUF is missing, so size falls back to 0 and the date to `created`.
    assert_eq!(first["size"], 0);
    assert_eq!(first["modified_at"], rfc3339(1_790_012_685));
    assert_eq!(first["details"]["format"], "gguf");
}

#[test]
fn digests_survive_the_ollama_cli() {
    // `ollama list` and `ollama ps` slice digest[:12] and panic when shorter.
    let tags = llama_as_tags(&router_listing());
    let a = tags["models"][0]["digest"].as_str().unwrap();
    let b = tags["models"][1]["digest"].as_str().unwrap();
    assert_eq!(a.len(), 64);
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
    assert_eq!(a, llama_as_tags(&router_listing())["models"][0]["digest"].as_str().unwrap());
}

#[test]
fn ps_lists_only_loaded_models() {
    let ps = llama_as_ps(&router_listing());
    assert_eq!(names(&ps, "models", "name"), ["ternary-bonsai-2-27b"]);
    assert!(ps["models"][0]["expires_at"].is_string());
}

#[test]
fn openai_listing_drops_the_router_extensions() {
    let v1 = llama_as_openai(&router_listing());
    assert_eq!(v1["object"], "list");
    let first = v1["data"][0].as_object().unwrap();
    let mut keys: Vec<&str> = first.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["created", "id", "object", "owned_by"]);
}

#[test]
fn listings_merge_with_ollama_ones() {
    let ollama = json!({ "models": [{ "name": "qwen3.8:27b" }] });
    let merged = merge_models(&[ollama, llama_as_tags(&router_listing())]);
    assert_eq!(merged["models"].as_array().unwrap().len(), 3);

    let ollama_v1 = json!({ "object": "list", "data": [{ "id": "qwen3.8:27b" }, { "id": "ternary-bonsai-8b" }] });
    let merged = merge_openai_models(&[ollama_v1, llama_as_openai(&router_listing())]);
    assert_eq!(
        names(&merged, "data", "id"),
        ["qwen3.8:27b", "ternary-bonsai-8b", "ternary-bonsai-2-27b"]
    );
}

#[test]
fn tolerates_a_listing_without_data() {
    assert!(llama_as_tags(&json!({}))["models"].as_array().unwrap().is_empty());
    assert!(llama_as_ps(&json!({ "error": "x" }))["models"].as_array().unwrap().is_empty());
}

#[test]
fn formats_rfc3339() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(rfc3339(1_790_012_685), "2026-09-21T17:44:45Z");
}
