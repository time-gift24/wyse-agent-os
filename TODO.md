# Stratum Deferred Capabilities

This ledger contains only capabilities that the current crates intentionally do
not provide. It is not an execution plan, milestone list, or record of completed
work.

## Web composition

- Web-owned Agent creation and single-writer identity, with creator-only write
  authorization and shared-reader authorization.
- Mounted filesystem root selection and ACL enforcement for each Agent.
- A Web recovery endpoint that subscribes to and buffers the retained stream
  before paging complete-message history through a fixed barrier, discards
  duplicate stable messages, and then continues the same live subscription.
- Recovery filtering that keeps unsequenced events only for the active run or a
  later buffered run and drops provisional deltas from inactive runs.
- Durable client `EventCursor` storage and an explicit cursor-reset response.

## Storage backends

- A durable, cross-process CAS-capable mounted filesystem backend selected by
  Web. `LocalFilesystem` remains a tool sandbox and does not provide that
  guarantee.

## Future protocol scope

- Workflow state and history formats when a workflow caller exists.
- Multi-writer Agent input if Web intentionally replaces the current
  single-writer ownership model.
- Run- or node-scoped retained-log subscriptions when non-Agent consumers need
  them.
