use std::sync::Arc;

use wyse_agent::Agent;
use wyse_core::{ModelId, ModelRef};
use wyse_infra::EventStreamBus;
use wyse_llm::{
    ApiKey, DeepSeekModel, DeepSeekProvider, DeepSeekThinking, LlmProvider,
    OpenAICompatibleProvider,
};
use wyse_tools::BuiltinToolRegistry;

use crate::DefaultAgentError;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

/// Builds the no-tool default agent for one supported model reference.
///
/// # Errors
///
/// Returns an error when the provider/model is unsupported or agent wiring is incomplete.
pub fn build_default_agent(
    event_bus: Arc<dyn EventStreamBus>,
    api_key: ApiKey,
    model: &ModelRef,
) -> Result<Agent, DefaultAgentError> {
    let llm_provider = build_llm_provider(api_key, model)?;

    Ok(Agent::builder()
        .name("default-agent")
        .system_prompt(DEFAULT_SYSTEM_PROMPT)
        .llm_provider(llm_provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(event_bus)
        .build()?)
}

fn build_llm_provider(
    api_key: ApiKey,
    model: &ModelRef,
) -> Result<Arc<dyn LlmProvider>, DefaultAgentError> {
    match model.provider() {
        "openai" => Ok(Arc::new(
            OpenAICompatibleProvider::new(OPENAI_BASE_URL, api_key, model.model().clone())
                .with_provider_name("openai"),
        )),
        "deepseek" => Ok(Arc::new(DeepSeekProvider::new(
            DEEPSEEK_BASE_URL,
            api_key,
            deepseek_model(model.model())?,
            DeepSeekThinking::Disabled,
        ))),
        provider => Err(DefaultAgentError::UnsupportedProvider {
            provider: provider.to_owned(),
        }),
    }
}

fn deepseek_model(model: &ModelId) -> Result<DeepSeekModel, DefaultAgentError> {
    match model.as_str() {
        "deepseek-v4-flash" => Ok(DeepSeekModel::V4Flash),
        "deepseek-v4-pro" => Ok(DeepSeekModel::V4Pro),
        _ => Err(DefaultAgentError::UnsupportedDeepSeekModel {
            model: model.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use wyse_llm::ApiKey;

    use super::{DefaultAgentError, build_llm_provider};

    #[test]
    fn openai_wiring_reports_canonical_provider_and_model() {
        let provider = build_llm_provider(
            ApiKey::new("test-key"),
            &"openai:gpt-4.1-mini".parse().expect("model ref parses"),
        )
        .expect("openai is supported");

        assert_eq!(provider.provider_name(), "openai");
        assert_eq!(provider.model_id().as_str(), "gpt-4.1-mini");
    }

    #[test]
    fn deepseek_wiring_reports_canonical_provider_and_model() {
        let provider = build_llm_provider(
            ApiKey::new("test-key"),
            &"deepseek:deepseek-v4-flash"
                .parse()
                .expect("model ref parses"),
        )
        .expect("deepseek model is supported");

        assert_eq!(provider.provider_name(), "deepseek");
        assert_eq!(provider.model_id().as_str(), "deepseek-v4-flash");
    }

    #[test]
    fn deepseek_wiring_rejects_unknown_model() {
        let error = match build_llm_provider(
            ApiKey::new("test-key"),
            &"deepseek:not-a-model".parse().expect("model ref parses"),
        ) {
            Ok(_) => panic!("unknown DeepSeek model should fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            DefaultAgentError::UnsupportedDeepSeekModel { .. }
        ));
    }
}
