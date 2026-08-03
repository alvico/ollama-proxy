use ollama_proxy::{run, Config};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config.json".into());

    let config = match Config::load(&path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("[proxy] {path}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if let Err(e) = run(config).await {
        eprintln!("[proxy] {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
