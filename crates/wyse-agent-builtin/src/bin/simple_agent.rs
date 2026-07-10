use std::{io::Write, sync::Arc};

use futures_util::StreamExt;
use wyse_agent::AgentError;
use wyse_agent_builtin::build_default_agent;
use wyse_core::{AgentEvent, ChatMessage, ModelId, ModelIdParseError, RuntimeEvent};
use wyse_infra::{
    EventStreamBus,
    event_stream_bus::{EventStreamBusError, InMemoryEventStreamBus},
};
use wyse_llm::{
    ApiKey, DeepSeekModel, DeepSeekProvider, DeepSeekThinking, LlmError, LlmProvider,
    LlmProviderManager, OpenAICompatibleProvider,
};

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

#[derive(serde::Deserialize)]
struct Config {
    openai: Option<ProviderConfig>,
    deepseek: Option<ProviderConfig>,
}

#[derive(serde::Deserialize)]
struct ProviderConfig {
    api_key: String,
    models: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
enum SimpleAgentError {
    #[error("failed to read config.toml")]
    ConfigRead(#[source] std::io::Error),
    #[error("failed to parse config.toml")]
    ConfigParse(#[source] toml::de::Error),
    #[error("usage: simple_agent --model provider:model <prompt>")]
    InvalidArguments,
    #[error("invalid model id")]
    ModelId(#[from] ModelIdParseError),
    #[error("provider manager operation failed")]
    ProviderManager(#[from] LlmError),
    #[error("unsupported deepseek model: {model}")]
    UnsupportedDeepSeekModel { model: ModelId },
    #[error("agent operation failed")]
    Agent(#[from] AgentError),
    #[error("event stream failed")]
    EventStream(#[from] EventStreamBusError),
    #[error("failed to encode event")]
    Encode(#[from] serde_json::Error),
    #[error("failed to write event")]
    Write(#[from] std::io::Error),
    #[error("agent run failed")]
    AgentFailed,
    #[error("agent run cancelled")]
    AgentCancelled,
    #[error("event stream closed before the agent finished")]
    StreamClosed,
}

fn parse_config(input: &str) -> Result<Config, SimpleAgentError> {
    toml::from_str(input).map_err(SimpleAgentError::ConfigParse)
}

fn parse_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(ModelId, String), SimpleAgentError> {
    if args.next().as_deref() != Some("--model") {
        return Err(SimpleAgentError::InvalidArguments);
    }

    let model = args
        .next()
        .ok_or(SimpleAgentError::InvalidArguments)?
        .parse()?;
    let prompt = args.next().ok_or(SimpleAgentError::InvalidArguments)?;
    if args.next().is_some() {
        return Err(SimpleAgentError::InvalidArguments);
    }

    Ok((model, prompt))
}

fn configured_providers(config: &Config) -> Result<LlmProviderManager, SimpleAgentError> {
    let mut providers = LlmProviderManager::new();

    if let Some(openai) = &config.openai {
        for model in &openai.models {
            let model = ModelId::new("openai", model)?;
            let provider: Arc<dyn LlmProvider> = Arc::new(OpenAICompatibleProvider::new(
                OPENAI_BASE_URL,
                ApiKey::new(&openai.api_key),
                model,
            ));
            providers.register(provider)?;
        }
    }

    if let Some(deepseek) = &config.deepseek {
        for model in &deepseek.models {
            let model = ModelId::new("deepseek", model)?;
            let provider: Arc<dyn LlmProvider> = Arc::new(DeepSeekProvider::new(
                DEEPSEEK_BASE_URL,
                ApiKey::new(&deepseek.api_key),
                deepseek_model(&model)?,
                DeepSeekThinking::Disabled,
            ));
            providers.register(provider)?;
        }
    }

    Ok(providers)
}

fn deepseek_model(model: &ModelId) -> Result<DeepSeekModel, SimpleAgentError> {
    match model.model_name() {
        "deepseek-v4-flash" => Ok(DeepSeekModel::V4Flash),
        "deepseek-v4-pro" => Ok(DeepSeekModel::V4Pro),
        _ => Err(SimpleAgentError::UnsupportedDeepSeekModel {
            model: model.clone(),
        }),
    }
}

#[tokio::main]
async fn main() -> Result<(), SimpleAgentError> {
    let (model, prompt) = parse_args(std::env::args().skip(1))?;
    let config = parse_config(
        &std::fs::read_to_string("config.toml").map_err(SimpleAgentError::ConfigRead)?,
    )?;
    let llm_provider = configured_providers(&config)?.get(&model)?;
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let event_bus: Arc<dyn EventStreamBus> = bus.clone();
    let agent = build_default_agent(event_bus, llm_provider)?;
    let run_id = agent.run_turn(ChatMessage::user(prompt)).await?;
    let mut stream = bus.subscribe_run(run_id).await?;
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();

    while let Some(envelope) = stream.next().await {
        let envelope = envelope?;
        serde_json::to_writer(&mut stdout, &envelope)?;
        writeln!(stdout)?;
        stdout.flush()?;

        match &envelope.event {
            RuntimeEvent::Agent {
                event: AgentEvent::Finished { .. },
                ..
            } => return Ok(()),
            RuntimeEvent::Agent {
                event: AgentEvent::Failed { .. },
                ..
            } => return Err(SimpleAgentError::AgentFailed),
            RuntimeEvent::Agent {
                event: AgentEvent::Cancelled,
                ..
            } => return Err(SimpleAgentError::AgentCancelled),
            _ => {}
        }
    }

    Err(SimpleAgentError::StreamClosed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parses_provider_keys_and_model_lists() {
        let config = parse_config("[openai]\napi_key = \"test\"\nmodels = [\"gpt-4.1-mini\"]")
            .expect("config parses");

        assert_eq!(
            config.openai.expect("openai config").models,
            ["gpt-4.1-mini"]
        );
    }

    #[test]
    fn arguments_require_model_flag_and_one_prompt() {
        let (model, prompt) = parse_args(
            ["--model", "openai:gpt-4.1-mini", "hello"]
                .into_iter()
                .map(str::to_owned),
        )
        .expect("arguments parse");

        assert_eq!(model.as_str(), "openai:gpt-4.1-mini");
        assert_eq!(prompt, "hello");
    }

    #[test]
    fn arguments_reject_any_form_other_than_model_and_one_prompt() {
        for arguments in [
            vec![],
            vec!["hello"],
            vec!["--model", "openai:gpt-4.1-mini"],
            vec!["--model", "openai:gpt-4.1-mini", "hello", "again"],
        ] {
            assert!(matches!(
                parse_args(arguments.into_iter().map(str::to_owned)),
                Err(SimpleAgentError::InvalidArguments)
            ));
        }
    }

    #[test]
    fn configured_providers_register_every_configured_model() {
        let config = parse_config(
            "[openai]\napi_key = \"test\"\nmodels = [\"gpt-4.1-mini\"]\n\
             [deepseek]\napi_key = \"test\"\nmodels = [\"deepseek-v4-flash\"]",
        )
        .expect("config parses");

        let providers = configured_providers(&config).expect("providers configure");

        assert_eq!(
            providers
                .get(&ModelId::new("openai", "gpt-4.1-mini").expect("model id parses"))
                .expect("openai provider registers")
                .model_id()
                .as_str(),
            "openai:gpt-4.1-mini"
        );
        assert_eq!(
            providers
                .get(&ModelId::new("deepseek", "deepseek-v4-flash").expect("model id parses"),)
                .expect("deepseek provider registers")
                .model_id()
                .as_str(),
            "deepseek:deepseek-v4-flash"
        );
    }

    #[test]
    fn configured_providers_reject_unknown_deepseek_model() {
        let config = parse_config("[deepseek]\napi_key = \"test\"\nmodels = [\"not-a-model\"]")
            .expect("config parses");

        let error = match configured_providers(&config) {
            Ok(_) => panic!("unknown DeepSeek model should reject"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            SimpleAgentError::UnsupportedDeepSeekModel { .. }
        ));
    }
}
