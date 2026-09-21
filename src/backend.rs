use crate::config::{Backend, BackendKind};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

/// Models currently held in memory by `backend`.
pub async fn resident(client: &Client, backend: &Backend) -> Vec<String> {
    let path = match backend.kind {
        BackendKind::Ollama => "/api/ps",
        BackendKind::LlamaServer => "/models",
    };
    let Ok(resp) = client.get(format!("{}{path}", backend.url)).send().await else {
        return Vec::new();
    };
    let Ok(body) = resp.json::<Value>().await else {
        return Vec::new();
    };
    match backend.kind {
        BackendKind::Ollama => model_names(&body),
        BackendKind::LlamaServer => llama_resident(&body),
    }
}

/// Drops `model` from memory now rather than when its keep-alive runs out.
pub async fn unload(client: &Client, backend: &Backend, model: &str) -> reqwest::Result<()> {
    match backend.kind {
        // `keep_alive: 0` with no prompt makes ollama drop the model immediately.
        BackendKind::Ollama => client
            .post(format!("{}/api/generate", backend.url))
            .json(&json!({ "model": model, "keep_alive": 0 }))
            .send()
            .await
            .map(|_| ()),
        // The router stops the child server holding the model, which is what
        // releases its VRAM; it autoloads it again on the next request.
        BackendKind::LlamaServer => client
            .post(format!("{}/models/unload", backend.url))
            .json(&json!({ "model": model }))
            .send()
            .await?
            .error_for_status()
            .map(|_| ()),
    }
}

/// Frees every model held by `backend`, returning how many were unloaded.
pub async fn drain(client: &Client, backend: &Backend) -> usize {
    let mut freed = 0;
    for model in resident(client, backend).await {
        match unload(client, backend, &model).await {
            Ok(()) => {
                log::info!("evicted {model} from {}", backend.url);
                freed += 1;
            }
            Err(e) => log::warn!("failed to evict {model} from {}: {e}", backend.url),
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

/// The `/v1/models` counterpart of [`merge_models`], keyed by `id`.
pub fn merge_openai_models(responses: &[Value]) -> Value {
    let mut seen = std::collections::HashSet::new();
    let data: Vec<Value> = responses
        .iter()
        .flat_map(|body| body["data"].as_array().cloned().unwrap_or_default())
        .filter(|m| seen.insert(m["id"].as_str().unwrap_or_default().to_owned()))
        .collect();

    json!({ "object": "list", "data": data })
}

/// A llama-server router's `/models` listing in the shape ollama returns from
/// `/api/tags`, so ollama clients see its models beside ollama's own.
pub fn llama_as_tags(body: &Value) -> Value {
    let models: Vec<Value> = llama_entries(body).map(ollama_entry).collect();
    json!({ "models": models })
}

/// The same for `/api/ps`: only the models the router has loaded.
pub fn llama_as_ps(body: &Value) -> Value {
    let models: Vec<Value> = llama_entries(body)
        .filter(|m| is_resident(m))
        .map(|m| {
            let mut entry = ollama_entry(m);
            entry["size_vram"] = entry["size"].clone();
            // The router never unloads on a timer, so the model stays until
            // evicted. ollama's CLI renders a far-off expiry as "Forever".
            entry["expires_at"] = json!("2318-01-01T00:00:00Z");
            entry
        })
        .collect();
    json!({ "models": models })
}

/// The router's `/v1/models` minus its extensions. Those include the full
/// command line of every child server -- host paths a client has no use for.
pub fn llama_as_openai(body: &Value) -> Value {
    let data: Vec<Value> = llama_entries(body)
        .map(|m| {
            json!({
                "id": m["id"],
                "object": "model",
                "created": m["created"],
                "owned_by": m["owned_by"],
            })
        })
        .collect();
    json!({ "object": "list", "data": data })
}

fn llama_resident(body: &Value) -> Vec<String> {
    llama_entries(body)
        .filter(|m| is_resident(m))
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect()
}

fn llama_entries(body: &Value) -> impl Iterator<Item = &Value> {
    body["data"].as_array().into_iter().flatten()
}

/// "loading" counts: a model mid-load is already claiming VRAM.
fn is_resident(model: &Value) -> bool {
    matches!(
        model["status"]["value"].as_str(),
        Some("loaded" | "loading")
    )
}

fn ollama_entry(model: &Value) -> Value {
    let id = model["id"].as_str().unwrap_or_default();
    // Size and date come from the GGUF itself, which is on this host because
    // the router serves loopback only. Best effort: a model the router fetched
    // by name has no --model argument, and it then lists as 0 bytes.
    let meta = gguf_path(model).and_then(|p| std::fs::metadata(p).ok());
    let size = meta.as_ref().map_or(0, |m| m.len());
    let modified = meta
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .or_else(|| model["created"].as_u64())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs())
        });

    json!({
        "name": id,
        "model": id,
        "modified_at": rfc3339(modified),
        "size": size,
        "digest": pseudo_digest(id),
        "details": { "format": "gguf", "family": "", "families": null,
                     "parameter_size": "", "quantization_level": "" },
    })
}

/// The router has no content digest, but ollama's CLI slices `digest[:12]`
/// unconditionally, so an empty one makes `ollama list` and `ollama ps` panic
/// for every model in the listing. A sha256-shaped value derived from the id is
/// enough: it only needs to be 64 hex digits and stable across calls.
fn pseudo_digest(id: &str) -> String {
    use std::hash::{Hash, Hasher};
    (0u8..4)
        .map(|salt| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (salt, id).hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        })
        .collect()
}

/// The model file a router child server was started with, from the command
/// line the router reports for it.
fn gguf_path(model: &Value) -> Option<&str> {
    let args = model["status"]["args"].as_array()?;
    let at = args.iter().position(|a| a == "--model" || a == "-m")?;
    args.get(at + 1)?.as_str()
}

/// Unix seconds as an RFC 3339 UTC timestamp, which ollama clients parse.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil conversion (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
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
