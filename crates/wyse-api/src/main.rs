//! Wyse API process entry point.

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init()?;
    wyse_api::run_from_path("config.toml").await?;
    Ok(())
}
