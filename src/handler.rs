use crate::{backend, options, state::AppState};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Map, Value};
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
    if parts.method == Method::GET && matches!(path, "/api/tags" | "/api/ps") {
        return aggregate(&state, path).await;
    }

    let model = serde_json::from_slice::<ModelField>(&body)
        .ok()
        .and_then(|m| m.model);

    let name = match &model {
        Some(model) => state.backend_for(model),
        None => &state.config.default_backend,
    };
    let Some(base) = state.url_of(name) else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("unknown backend '{name}'"),
        );
    };
    let (name, base) = (name.to_owned(), base.to_owned());

    let mut body = body.to_vec();

    // Only model-bearing requests carry options, and only they justify evicting
    // the other backend.
    if let Some(model) = &model {
        let empty = Map::new();
        let fields = state.defaults_for(model).unwrap_or(&empty);
        let defaults = state.options_for(model).unwrap_or(&empty);
        if let Some(merged) = options::merge(&body, fields, defaults) {
            log::debug!("{model}: defaulted {}", merged.applied.join(", "));
            body = merged.body;
        }
        log::debug!("{} {path} model={model} -> '{name}'", parts.method);
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

async fn aggregate(state: &AppState, path: &str) -> Response {
    let mut bodies = Vec::new();
    for base in state.config.backends.values() {
        if let Ok(resp) = state.client.get(format!("{base}{path}")).send().await {
            if let Ok(body) = resp.json::<Value>().await {
                bodies.push(body);
            }
        }
    }
    axum::Json(backend::merge_models(&bodies)).into_response()
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
    log::error!("{message}");
    (status, message).into_response()
}
