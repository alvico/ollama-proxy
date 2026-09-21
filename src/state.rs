use crate::{
    backend,
    config::{Backend, Config},
    routing,
};
use reqwest::Client;
use serde_json::{Map, Value};
use std::time::Duration;
use tokio::sync::Mutex;

/// Model loads can take minutes on a partially offloaded 28GB model, so the
/// client must not impose a short deadline.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(3600);

pub struct AppState {
    pub config: Config,
    pub client: Client,
    active: Mutex<Option<String>>,
}

impl AppState {
    pub fn new(config: Config) -> reqwest::Result<Self> {
        let client = Client::builder().timeout(UPSTREAM_TIMEOUT).build()?;
        Ok(Self {
            config,
            client,
            active: Mutex::new(None),
        })
    }

    pub fn backend_for(&self, model: &str) -> &str {
        routing::resolve(
            &self.config.routes,
            &self.config.default_backend,
            model,
        )
    }

    pub fn backend(&self, name: &str) -> Option<&Backend> {
        self.config.backends.get(name)
    }

    pub fn options_for(&self, model: &str) -> Option<&Map<String, Value>> {
        routing::longest_prefix(&self.config.options, model)
    }

    pub fn defaults_for(&self, model: &str) -> Option<&Map<String, Value>> {
        routing::longest_prefix(&self.config.defaults, model)
    }

    /// Makes `target` the only backend holding a model. There is not enough
    /// VRAM for two backends to stay loaded, so the others are drained first.
    pub async fn activate(&self, target: &str) {
        let mut active = self.active.lock().await;
        if active.as_deref() == Some(target) {
            return;
        }

        for (name, other) in &self.config.backends {
            if name != target {
                backend::drain(&self.client, other).await;
            }
        }

        log::info!("active backend -> '{target}'");
        *active = Some(target.to_owned());
    }
}
