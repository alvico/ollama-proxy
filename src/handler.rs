use crate::{backend, config::BackendKind, options, state::AppState};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::sync::Arc;

const MAX_BODY: usize = 256 * 1024 * 1024;

/// Every ollama endpoint that loads a model names it in a `model` field.
#[derive(Deserialize)]
struct ModelField {
    model: Option<String>,
}

pub async fn proxy(State(state): State<Arc<AppState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();

    let body = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(e) => return error(StatusCode::BAD_REQUEST, format!("unreadable body: {e}")),
    };

    let path = parts.uri.path();
    if parts.method == Method::GET && matches!(path, "/api/tags" | "/api/ps" | "/v1/models") {
        return aggregate(&state, path).await;
    }

    let model = serde_json::from_slice::<ModelField>(&body)
        .ok()
        .and_then(|m| m.model);

    let name = match &model {
        Some(model) => state.backend_for(model),
        None => &state.config.default_backend,
    };
    let Some(target) = state.backend(name) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unknown backend '{name}'"),
        );
    };

    // llama-server has no /api/* at all, so forwarding would only get the
    // client a bare 404 that reads as "model not found". Say what does work,
    // in ollama's error shape so ollama clients display it.
    if target.kind == BackendKind::LlamaServer && !path.starts_with("/v1/") {
        let message = format!(
            "{} is served by llama-server backend '{name}', which speaks only the \
             OpenAI API: use /v1/chat/completions instead of {path}",
            model.as_deref().unwrap_or("this model"),
        );
        log::info!("{message}");
        return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": message }))).into_response();
    }

    let (name, base) = (name.to_owned(), target.url.clone());

    let mut body = body.to_vec();

    // Only model-bearing requests carry options, and only they justify evicting
    // the other backend.
    if let Some(model) = &model {
        const PREVIEW: usize = 2048;
        let preview: String = String::from_utf8_lossy(&body[..body.len().min(PREVIEW)]).into_owned();
        log::debug!(
            "{} {path} model={model} -> '{name}' body_bytes={} preview={preview}",
            parts.method,
            body.len()
        );
        let empty = Map::new();
        let fields = state.defaults_for(model).unwrap_or(&empty);
        let defaults = state.options_for(model).unwrap_or(&empty);
        if let Some(merged) =
            options::merge(&body, fields, defaults, &state.config.exclusive_defaults)
        {
            log::debug!("{model}: defaulted {}", merged.applied.join(", "));
            body = merged.body;
        }
        state.activate(&name).await;
    }

    forward(&state, &parts, body, &base, &name).await
}

async fn forward(
    state: &AppState,
    parts: &axum::http::request::Parts,
    body: Vec<u8>,
    base: &str,
    name: &str,
) -> Response {
    let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let url = format!("{base}{}{query}", parts.uri.path());

    let mut request = state.client.request(parts.method.clone(), url);
    for (key, value) in &parts.headers {
        if key != header::HOST && key != header::CONTENT_LENGTH {
            request = request.header(key, value);
        }
    }

    let upstream = match request.body(body).send().await {
        Ok(resp) => resp,
        Err(e) => return error(StatusCode::BAD_GATEWAY, format!("backend '{name}': {e}")),
    };

    let status = upstream.status();
    let headers = relay_headers(upstream.headers());

    // ollama emits NDJSON token by token; buffering here would break streaming.
    let stream = upstream
        .bytes_stream()
        .map(|chunk| chunk.map_err(std::io::Error::other));

    (status, headers, Body::from_stream(stream)).into_response()
}

/// One listing across every backend. A llama-server is asked for its own
/// `/models` and translated, since it has neither `/api/tags` nor `/api/ps`.
/// A backend that is down is left out rather than failing the whole listing.
async fn aggregate(state: &AppState, path: &str) -> Response {
    let mut bodies = Vec::new();
    for target in state.config.backends.values() {
        let source = match target.kind {
            BackendKind::Ollama => path,
            BackendKind::LlamaServer => "/models",
        };
        let Ok(resp) = state.client.get(format!("{}{source}", target.url)).send().await else {
            continue;
        };
        let Ok(body) = resp.json::<Value>().await else {
            continue;
        };
        bodies.push(match (target.kind, path) {
            (BackendKind::Ollama, _) => body,
            (BackendKind::LlamaServer, "/api/tags") => backend::llama_as_tags(&body),
            (BackendKind::LlamaServer, "/api/ps") => backend::llama_as_ps(&body),
            (BackendKind::LlamaServer, _) => backend::llama_as_openai(&body),
        });
    }
    let merged = if path == "/v1/models" {
        backend::merge_openai_models(&bodies)
    } else {
        backend::merge_models(&bodies)
    };
    axum::Json(merged).into_response()
}

/// Framing headers belong to this connection, not the upstream one.
fn relay_headers(upstream: &HeaderMap) -> HeaderMap {
    upstream
        .iter()
        .filter(|(key, _)| **key != header::TRANSFER_ENCODING && **key != header::CONTENT_LENGTH)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn error(status: StatusCode, message: String) -> Response {
    log::info!("{message}");
    (status, message).into_response()
}
