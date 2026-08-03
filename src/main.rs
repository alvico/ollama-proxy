use ollama_proxy::{run, Config};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .init();

    let path = std::env::args().nth(1).unwrap_or_else(|| "config.json".into());

    let config = match Config::load(&path) {
        Ok(config) => config,
        Err(e) => {
            log::error!("{path}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(e) = run(config).await {
        log::error!("{e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
