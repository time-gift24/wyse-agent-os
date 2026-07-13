use std::error::Error;

use stratum_core::ModelId;
use stratum_llm::{
    ApiKey, ChatMessage, ChatRequest, ChatRole, LlmProvider, OpenAICompatibleProvider,
};

#[tokio::test]
#[ignore = "requires STRATUM_LLM_TEST_BASE_URL, STRATUM_LLM_TEST_API_KEY, and STRATUM_LLM_TEST_MODEL"]
async fn openai_compatible_provider_returns_chat_response() -> Result<(), Box<dyn Error>> {
    let base_url = std::env::var("STRATUM_LLM_TEST_BASE_URL")?;
    let api_key = ApiKey::new(std::env::var("STRATUM_LLM_TEST_API_KEY")?);
    let model: ModelId = std::env::var("STRATUM_LLM_TEST_MODEL")?.parse()?;
    let provider = OpenAICompatibleProvider::new(base_url, api_key, model.clone());

    let response = provider
        .chat(ChatRequest::new(model).with_message(ChatMessage::user("Say ok.")))
        .await?;

    assert_eq!(response.message.role, ChatRole::Assistant);

    Ok(())
}
