# wyse-store invariants

- The store commits an agent's `ModelConfig` only as part of the durable start transition that
  accepts the turn; a failed candidate must leave the previous snapshot intact.
- Legacy state without a model configuration is accepted solely to migrate it to the current
  persisted shape, never as a normal runtime state.
