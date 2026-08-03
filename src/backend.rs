use reqwest::Client;
use serde_json::{json, Value};

/// Models currently held in memory by the backend at `base`.
pub async fn resident(client: &Client, base: &str) -> Vec<String> {
    let Ok(resp) = client.get(format!("{base}/api/ps")).send().await else {
        return Vec::new();
    };
    let Ok(body) = resp.json::<Value>().await else {
        return Vec::new();
    };
    model_names(&body)
}

/// `keep_alive: 0` with no prompt makes ollama drop the model immediately.
pub async fn unload(client: &Client, base: &str, model: &str) -> reqwest::Result<()> {
    client
        .post(format!("{base}/api/generate"))
        .json(&json!({ "model": model, "keep_alive": 0 }))
        .send()
        .await
        .map(|_| ())
}

/// Frees every model held by `base`, returning how many were unloaded.
pub async fn drain(client: &Client, base: &str) -> usize {
    let mut freed = 0;
    for model in resident(client, base).await {
        match unload(client, base, &model).await {
            Ok(()) => {
                log::info!("evicted {model} from {base}");
                freed += 1;
            }
            Err(e) => log::warn!("failed to evict {model} from {base}: {e}"),
        }
    }
    freed
}

/// Merges the `models` arrays returned by each backend, keeping first-seen
/// entries. Backends share one model store, so their listings overlap.
pub fn merge_models(responses: &[Value]) -> Value {
    let mut seen = std::collections::HashSet::new();
    let models: Vec<Value> = responses
        .iter()
        .flat_map(|body| body["models"].as_array().cloned().unwrap_or_default())
        .filter(|m| seen.insert(m["name"].as_str().unwrap_or_default().to_owned()))
        .collect();

    json!({ "models": models })
}

fn model_names(body: &Value) -> Vec<String> {
    body["models"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|m| m["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
