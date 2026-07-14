# stratum-store invariants

## Legacy Model Configuration

- The store commits an agent's `ModelConfig` only as part of the durable start transition that
  accepts the turn; a failed candidate must leave the previous snapshot intact.
- Legacy state without a model configuration is accepted solely to migrate it to the current
  persisted shape, never as a normal runtime state.

## Agent Loop Event Projection

- Persistence is a durable-event consumer; it is not an `AgentLoop` dependency.
- The store-backed consumer applies durable loop-event projections before
  acknowledging or forwarding the event.
- `IterationCompleted` calls `AgentStore::complete_iteration` before forwarding.
- A store or projection failure returns without acknowledgement and without
  forwarding the event.
- After a projection commits successfully, forwarding is best-effort. A
  forwarding error or timeout cannot undo the store commit.
- Durable events without a store projection retain the downstream bus's
  acknowledgement requirement.
