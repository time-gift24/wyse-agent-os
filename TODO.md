# Wyse Agent OS TODO

## Crates

- [ ] `wyse-core`
  - Core IDs: `RunId`, `AgentId`, `ToolId`, `ModelId`
  - Shared error/result types
  - Common value model and JSON schema helpers

- [ ] `wyse-infra`
  - Internal event bus
  - Runtime event and trace primitives
  - Clock/time provider
  - ID generator
  - Cancellation token helpers
  - Retry/backoff utilities
  - Rate-limit primitives
  - Permission context and policy hooks
  - Config/env loading helpers

- [ ] `wyse-llm`
  - Unified chat, completion, streaming, embedding, rerank abstractions
  - OpenAI-compatible provider first
  - Mock provider for tests
  - Tool-call normalization
  - Structured output support
  - Token/cost primitives

- [ ] `wyse-tools`
  - Internal `Tool` trait
  - Tool registry
  - Tool input/output schema validation
  - Local tool adapter
  - HTTP/OpenAPI tool adapter
  - Tool execution trace and errors

- [ ] `wyse-mcp`
  - MCP protocol types
  - MCP client
  - MCP server
  - MCP transport support: stdio first, HTTP later
  - Adapter from MCP tools to `wyse-tools`
  - Adapter from Wyse tools/workflows/agents to MCP server tools
  - MCP permission, allowlist, timeout, audit hooks

- [ ] `wyse-agent`
  - Agent runtime loop
  - Function-calling agent strategy first
  - ReAct strategy later
  - Tool-use orchestration through `wyse-tools`
  - Model access through `wyse-llm`
  - Step budget, timeout, cancellation
  - Agent memory interface

- [ ] `wyse-workflow`
  - Workflow DSL types
  - Workflow node IDs and edge references
  - DAG validation
  - Node runtime trait
  - In-memory graph executor first
  - Queue-based executor later
  - Built-in nodes: start, output, llm, tool, agent, if, loop, parallel, human-input
  - Run state, node state, resume/cancel support

- [ ] `wyse-knowledge`
  - Document ingestion pipeline
  - Parser/chunker interfaces
  - Embedding through `wyse-llm`
  - Vector store abstraction
  - Retriever and reranker
  - Citation/source metadata

- [ ] `wyse-store`
  - Persistence traits
  - Workflow definitions
  - Run snapshots
  - Event log
  - Secrets references
  - SQLite first, Postgres later

- [ ] `wyse-api`
  - Axum HTTP API
  - Workflow run API
  - Agent invoke API
  - SSE streaming events
  - Human-input resume API
  - Admin/debug endpoints

- [ ] `wyse-cli`
  - Validate workflow files
  - Run workflow locally
  - Invoke agent locally
  - List tools/models/MCP servers
  - Debug run events

## Initial Implementation Order

1. [ ] Create Rust workspace and crate skeletons
2. [ ] Implement `wyse-core`
3. [ ] Implement `wyse-infra`
4. [ ] Implement `wyse-llm` with mock + OpenAI-compatible provider
5. [ ] Implement `wyse-tools` with local tools
6. [ ] Implement `wyse-mcp` client with stdio tool adapter
7. [ ] Implement `wyse-agent` function-calling loop
8. [ ] Implement `wyse-workflow` in-memory DAG runtime
9. [ ] Implement `wyse-store` SQLite persistence
10. [ ] Implement `wyse-api` REST + SSE
11. [ ] Implement `wyse-cli` commands
12. [ ] Implement `wyse-knowledge`
13. [ ] Add MCP server mode

## First Runnable Milestone

- [ ] `wyse-cli chat --model <model>`
- [ ] `wyse-cli tool call <tool> --json <args>`
- [ ] `wyse-cli mcp list-tools <server>`
- [ ] `wyse-cli agent run <agent.yaml>`
- [ ] `wyse-cli workflow run <workflow.yaml>`
