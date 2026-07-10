use std::sync::Arc;

use wyse_agent::{Agent, AgentError};
use wyse_infra::EventStreamBus;
use wyse_llm::LlmProvider;
use wyse_tools::BuiltinToolRegistry;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";

/// Builds the no-tool default agent with an injected provider.
///
/// # Errors
///
/// Returns an error when the supplied agent wiring is incomplete.
pub fn build_default_agent(
    event_bus: Arc<dyn EventStreamBus>,
    llm_provider: Arc<dyn LlmProvider>,
) -> Result<Agent, AgentError> {
    Agent::builder()
        .name("default-agent")
        .system_prompt(DEFAULT_SYSTEM_PROMPT)
        .llm_provider(llm_provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(event_bus)
        .build()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wyse_agent::AgentError;
    use wyse_infra::event_stream_bus::InMemoryEventStreamBus;
    use wyse_llm::MockLlmProvider;

    use super::build_default_agent;

    #[test]
    fn build_default_agent_returns_agent_error() {
        let result: Result<_, AgentError> = build_default_agent(
            Arc::new(InMemoryEventStreamBus::default()),
            Arc::new(MockLlmProvider::new()),
        );

        assert!(result.is_ok());
    }
}
