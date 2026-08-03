use crate::{config::Config, handler, state::AppState};
use axum::{routing::any, Router};
use std::{io, sync::Arc};

pub async fn run(config: Config) -> io::Result<()> {
    describe(&config);

    let listen = config.listen.clone();
    let state = Arc::new(AppState::new(config).map_err(io::Error::other)?);
    let app = Router::new().fallback(any(handler::proxy)).with_state(state);

    let listener = tokio::net::TcpListener::bind(&listen).await?;
    axum::serve(listener, app).await
}

fn describe(config: &Config) {
    log::info!("listening on {}", config.listen);
    for (name, base) in &config.backends {
        log::info!("backend '{name}' -> {base}");
    }
    for (prefix, backend) in &config.routes {
        log::info!("route '{prefix}*' -> '{backend}'");
    }
    log::info!("everything else -> '{}'", config.default_backend);
    for (prefix, options) in &config.options {
        let rendered: Vec<String> = options.iter().map(|(k, v)| format!("{k}={v}")).collect();
        log::info!("options '{prefix}*' <- {}", rendered.join(" "));
    }
    for (prefix, fields) in &config.defaults {
        let rendered: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={v}")).collect();
        log::info!("defaults '{prefix}*' <- {}", rendered.join(" "));
    }
}
