use std::error::Error;

use stratum_llm::{
    ApiKey, ChatMessage, ChatRequest, ChatRole, DeepSeekModel, DeepSeekProvider, DeepSeekThinking,
    LlmProvider,
};

#[tokio::test]
#[ignore = "requires STRATUM_LLM_TEST_BASE_URL, STRATUM_LLM_TEST_API_KEY, and STRATUM_LLM_TEST_MODEL"]
async fn deepseek_provider_returns_chat_response() -> Result<(), Box<dyn Error>> {
    let base_url = std::env::var("STRATUM_LLM_TEST_BASE_URL")?;
    let api_key = ApiKey::new(std::env::var("STRATUM_LLM_TEST_API_KEY")?);
    let model = match std::env::var("STRATUM_LLM_TEST_MODEL")?.as_str() {
        "deepseek-v4-flash" => DeepSeekModel::V4Flash,
        "deepseek-v4-pro" => DeepSeekModel::V4Pro,
        _ => {
            return Err(
                "STRATUM_LLM_TEST_MODEL must be deepseek-v4-flash or deepseek-v4-pro".into(),
            );
        }
    };
    let provider = DeepSeekProvider::new(base_url, api_key, model, DeepSeekThinking::Disabled);

    let response = provider
        .chat(ChatRequest::new(model.model_id()).with_message(ChatMessage::user("Say ok.")))
        .await?;

    assert_eq!(response.message.role, ChatRole::Assistant);

    Ok(())
}
