//! HTTP API host for persisted Wyse agents.

mod api;
mod error;
mod host;

use std::{path::Path, sync::Arc};

use wyse_config::{Config, ProviderConfig};
use wyse_core::ModelId;
use wyse_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig};
use wyse_infra::{EventStreamBus, NatsEventStreamBusConfig, create_nats_event_stream_bus};
use wyse_llm::{
    ApiKey, DeepSeekModel, DeepSeekProvider, DeepSeekThinking, LlmProviderManager,
    OpenAICompatibleProvider,
};

pub use api::{AgentCreated, AgentView, RunAccepted, router};
pub use error::{AgentCleanupError, HostError};
pub use host::{HostState, HostedAgent};

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

/// Reads a config file and serves until shutdown.
///
/// # Errors
///
/// Returns [`HostError`] when the file, configuration, runtime dependencies, listener, or server
/// fails.
pub async fn run_from_path(path: impl AsRef<Path>) -> Result<(), HostError> {
    let contents = tokio::fs::read_to_string(path).await?;
    let config = Config::parse(&contents)?;
    config.require_api()?;
    config.require_nats()?;
    serve(config).await
}

/// Composes the configured providers, filesystem, NATS bus, restored host, and HTTP listener.
///
/// # Errors
///
/// Returns [`HostError`] when configuration or any runtime dependency cannot be initialized.
pub async fn serve(config: Config) -> Result<(), HostError> {
    let api = config.require_api()?.clone();
    let nats = NatsEventStreamBusConfig::try_from(config.require_nats()?)?;
    let providers = providers(&config)?;
    tokio::fs::create_dir_all(config.agent.storage_root.join("templates")).await?;
    tokio::fs::create_dir_all(config.agent.storage_root.join("history")).await?;
    let filesystem: Arc<dyn Filesystem> = Arc::new(LocalFilesystem::new(LocalFilesystemConfig {
        root: config.agent.storage_root.clone(),
        max_file_bytes: None,
    })?);
    let event_bus: Arc<dyn EventStreamBus> = Arc::new(create_nats_event_stream_bus(nats).await?);
    let state = HostState::restore(config, filesystem, event_bus, providers).await?;
    let listener = tokio::net::TcpListener::bind(api.bind).await?;
    let shutdown = state.shutdown_token();
    let server = axum::serve(listener, router(Arc::clone(&state)))
        .with_graceful_shutdown(shutdown.cancelled_owned());
    let mut server = Box::pin(async move { server.await });
    let mut signal_result = None;
    let server_result = tokio::select! {
        result = &mut server => Some(result),
        signal = tokio::signal::ctrl_c() => {
            signal_result = Some(signal);
            None
        }
    };
    state.shutdown().await;
    if let Some(signal_result) = signal_result {
        signal_result?;
        server.await?;
    } else if let Some(server_result) = server_result {
        server_result?;
    }
    Ok(())
}

fn providers(config: &Config) -> Result<LlmProviderManager, HostError> {
    let mut providers = LlmProviderManager::new();
    if let Some(provider) = &config.llm.openai {
        register_openai(&mut providers, provider)?;
    }
    if let Some(provider) = &config.llm.deepseek {
        register_deepseek(&mut providers, provider)?;
    }
    Ok(providers)
}

fn register_openai(
    providers: &mut LlmProviderManager,
    config: &ProviderConfig,
) -> Result<(), HostError> {
    for model in &config.models {
        let model_id = model_id("openai", model)?;
        providers.register(Arc::new(OpenAICompatibleProvider::new(
            OPENAI_BASE_URL,
            ApiKey::new(config.api_key.clone()),
            model_id,
        )))?;
    }
    Ok(())
}

fn register_deepseek(
    providers: &mut LlmProviderManager,
    config: &ProviderConfig,
) -> Result<(), HostError> {
    for model in &config.models {
        let adapter_model = match model.as_str() {
            "deepseek-v4-flash" => DeepSeekModel::V4Flash,
            "deepseek-v4-pro" => DeepSeekModel::V4Pro,
            _ => {
                return Err(HostError::UnsupportedDeepSeekModel {
                    model: model_id("deepseek", model)?,
                });
            }
        };
        providers.register(Arc::new(DeepSeekProvider::new(
            DEEPSEEK_BASE_URL,
            ApiKey::new(config.api_key.clone()),
            adapter_model,
            DeepSeekThinking::Disabled,
        )))?;
    }
    Ok(())
}

fn model_id(provider: &'static str, model: &str) -> Result<ModelId, HostError> {
    ModelId::new(provider, model).map_err(|source| HostError::InvalidConfiguredModel {
        provider,
        model: model.to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::{HostError, providers};
    use wyse_config::Config;
    use wyse_core::ModelId;

    #[test]
    fn registers_every_configured_openai_and_deepseek_model() {
        let config = Config::parse(
            r#"
[agent]
storage_root = "."

[llm]
default = "openai:gpt-4.1-mini"

[llm.openai]
api_key = "openai-key"
models = ["gpt-4.1-mini", "gpt-4.1"]

[llm.deepseek]
api_key = "deepseek-key"
models = ["deepseek-v4-flash", "deepseek-v4-pro"]
"#,
        )
        .expect("config parses");

        let providers = providers(&config).expect("providers compose");

        for model in [
            "openai:gpt-4.1-mini",
            "openai:gpt-4.1",
            "deepseek:deepseek-v4-flash",
            "deepseek:deepseek-v4-pro",
        ] {
            let model: ModelId = model.parse().expect("model id parses");
            assert_eq!(
                providers.get(&model).expect("provider exists").model_id(),
                model
            );
        }
    }

    #[test]
    fn rejects_deepseek_models_not_supported_by_the_adapter() {
        let config = Config::parse(
            r#"
[agent]
storage_root = "."

[llm]
default = "deepseek:deepseek-v4-flash"

[llm.deepseek]
api_key = "deepseek-key"
models = ["deepseek-v4-flash", "deepseek-v3"]
"#,
        )
        .expect("config parses");

        assert!(matches!(
            providers(&config),
            Err(HostError::UnsupportedDeepSeekModel { .. })
        ));
    }
}
