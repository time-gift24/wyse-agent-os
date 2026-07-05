# wyse-llm AGENTS.md

## Scope

`wyse-llm` owns Wyse LLM domain types, the mock provider, and low-level provider protocol adapters.

## Design Rules

- Keep one public provider trait until another real trait split has multiple implementations.
- Treat `protocol::openai_compatible` as a low-level protocol adapter, not the long-term provider model.
- Use `wyse_core::ModelId` for public model identity.
- `OpenAICompatibleProvider` is bound to one model; reject requests whose `ChatRequest.model` differs from the provider model.
- Map future OpenAI and Anthropic runtime output around one `LlmCallId`; do not add `model_id`, `message_id`, or message lifecycle events to runtime events.
- Use runtime `TextDelta.role` only for normal `system`, `user`, `assistant`, and `tool` text; keep reasoning as `ReasoningDelta`.
- Do not add provider registry, factory, manager, embedding, rerank, or Anthropic-compatible protocol without a concrete caller.
- Do not add DeepSeek, zhipu, or other provider-specific forks until a real compatibility difference needs code.
- Do not log prompts, completions, tool arguments, API keys, or raw provider payloads.
- Tool schema validation belongs in `wyse-tools`, not here.
