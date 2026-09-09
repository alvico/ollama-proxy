use crate::{config::Config, handler, state::AppState};
use axum::{routing::any, Router};
use std::{io, sync::Arc};
use tokio::task::JoinSet;

pub async fn run(config: Config) -> io::Result<()> {
    describe(&config);

    let addresses = config.listen_addresses();
    let state = Arc::new(AppState::new(config).map_err(io::Error::other)?);
    let app = Router::new().fallback(any(handler::proxy)).with_state(state);

    // One address failing to bind is logged rather than fatal. An address on a
    // VPN or other on-demand interface does not exist until that interface is
    // up, and losing it must not also take down the loopback listener that
    // local clients depend on.
    let mut serving = JoinSet::new();
    for address in &addresses {
        match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => {
                log::info!("listening on {address}");
                let app = app.clone();
                serving.spawn(async move { axum::serve(listener, app).await });
            }
            Err(e) => log::warn!("cannot bind {address}: {e}"),
        }
    }

    if serving.is_empty() {
        return Err(io::Error::other(format!(
            "no listen address could be bound ({})",
            addresses.join(", ")
        )));
    }

    // A listener dying ends the process rather than leaving the proxy
    // half-reachable, which a client cannot detect; start-stack.ps1 reports it.
    while let Some(finished) = serving.join_next().await {
        finished.map_err(io::Error::other)??;
    }
    Ok(())
}

fn describe(config: &Config) {
    // The bind addresses are logged as they succeed, not here, so the log shows
    // what is actually listening rather than what was asked for.
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
