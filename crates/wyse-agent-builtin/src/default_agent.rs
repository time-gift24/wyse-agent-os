use std::sync::Arc;

use wyse_agent::{Agent, AgentError};
use wyse_core::AgentId;
use wyse_infra::EventStreamBus;
use wyse_llm::LlmProvider;
use wyse_store::AgentStore;
use wyse_tools::BuiltinToolRegistry;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";

/// Builds the no-tool default agent with an injected provider.
///
/// # Errors
///
/// Returns an error when the supplied agent wiring is incomplete.
pub fn build_default_agent(
    agent_id: AgentId,
    store: Arc<dyn AgentStore>,
    event_bus: Arc<dyn EventStreamBus>,
    llm_provider: Arc<dyn LlmProvider>,
) -> Result<Agent, AgentError> {
    Agent::builder()
        .id(agent_id)
        .name("default-agent")
        .system_prompt(DEFAULT_SYSTEM_PROMPT)
        .llm_provider(llm_provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(event_bus)
        .store(store)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    type DefaultAgentBuilder = fn(
        AgentId,
        Arc<dyn AgentStore>,
        Arc<dyn EventStreamBus>,
        Arc<dyn LlmProvider>,
    ) -> Result<Agent, AgentError>;

    #[test]
    fn default_agent_builder_accepts_store_injection() {
        let _builder: DefaultAgentBuilder = build_default_agent;
    }
}
