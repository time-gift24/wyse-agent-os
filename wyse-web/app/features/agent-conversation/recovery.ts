import type { ConversationAction } from "~/features/agent-conversation/types"
import { ApiError, type StreamEnvelope, type WyseApi } from "~/lib/wyse-api"
import { subscribeToAgentEvents, type SseEvent } from "~/lib/wyse-event-stream"

const HISTORY_PAGE_LIMIT = 256

export type RecoveryDependencies = {
  api: Pick<WyseApi, "getAgent" | "getHistory">
  subscribe: typeof subscribeToAgentEvents
  loadCursor(agentId: string): string | undefined
  saveCursor(agentId: string, cursor: string): void
  clearCursor(agentId: string): void
  dispatch(action: ConversationAction): void
}

export async function recoverConversation(
  dependencies: RecoveryDependencies,
  input: { agentId: string; signal: AbortSignal }
): Promise<void> {
  let afterCursor = dependencies.loadCursor(input.agentId)
  let retriedExpiredCursor = false

  while (!input.signal.aborted) {
    try {
      await recoverOnce(dependencies, input, afterCursor)
      return
    } catch (error) {
      if (input.signal.aborted) return

      if (!retriedExpiredCursor && isExpiredStreamCursor(error)) {
        dependencies.clearCursor(input.agentId)
        afterCursor = undefined
        retriedExpiredCursor = true
        continue
      }

      dependencies.dispatch({
        type: "connection_error",
        error: connectionError(error),
      })
      return
    }
  }
}

async function recoverOnce(
  dependencies: RecoveryDependencies,
  input: { agentId: string; signal: AbortSignal },
  afterCursor: string | undefined
): Promise<void> {
  const buffered: BufferedEnvelope[] = []
  let ready = false
  let streamCompletion: StreamCompletion | undefined

  const accept = (bufferedEnvelope: BufferedEnvelope) => {
    dependencies.dispatch({
      type: "envelope_received",
      envelope: bufferedEnvelope.envelope,
    })
    if (bufferedEnvelope.cursor !== null)
      dependencies.saveCursor(input.agentId, bufferedEnvelope.cursor)
  }

  dependencies.dispatch({ type: "recovery_started", agentId: input.agentId })
  const subscription = dependencies.subscribe({
    // The hook binds the configured base URL before this reaches fetch.
    baseUrl: "",
    agentId: input.agentId,
    afterCursor,
    signal: input.signal,
    onEvent: (event) => {
      const envelope = parseEnvelope(event)
      if (envelope?.event.data.agent_id !== input.agentId) return

      const received = { envelope, cursor: event.id }
      if (ready) accept(received)
      else buffered.push(received)
    },
  })

  void subscription.done.then(
    () => {
      const completion: StreamCompletion = { type: "stream_ended" }
      if (input.signal.aborted) return
      if (ready) {
        dependencies.dispatch({
          type: "connection_error",
          error: connectionError(completion),
        })
      } else {
        streamCompletion = completion
      }
    },
    (error: unknown) => {
      const completion: StreamCompletion = { type: "stream_failed", error }
      if (input.signal.aborted) return
      if (ready) {
        dependencies.dispatch({
          type: "connection_error",
          error: connectionError(completion),
        })
      } else {
        streamCompletion = completion
      }
    }
  )

  const view = await dependencies.api.getAgent(input.agentId)
  throwIfAborted(input.signal)
  throwIfStreamCompleted(streamCompletion)
  dependencies.dispatch({ type: "view_loaded", view })

  let afterSeq = 0
  let hasMore = true
  while (hasMore) {
    const page = await dependencies.api.getHistory(input.agentId, {
      afterSeq,
      throughSeq: view.last_seq,
      limit: HISTORY_PAGE_LIMIT,
    })
    throwIfAborted(input.signal)
    throwIfStreamCompleted(streamCompletion)
    dependencies.dispatch({ type: "history_loaded", events: page.events })
    afterSeq = page.next_front_seq
    hasMore = page.has_more
  }

  for (const received of buffered) {
    if (
      received.envelope.business_seq !== undefined &&
      received.envelope.business_seq <= view.last_seq
    )
      continue
    accept(received)
  }

  throwIfStreamCompleted(streamCompletion)
  ready = true
  dependencies.dispatch({ type: "recovery_ready" })
}

type BufferedEnvelope = { envelope: StreamEnvelope; cursor: string | null }
type StreamCompletion =
  | { type: "stream_ended" }
  | { type: "stream_failed"; error: unknown }

function parseEnvelope(event: SseEvent): StreamEnvelope | undefined {
  try {
    const value: unknown = JSON.parse(event.data)
    return isStreamEnvelope(value) ? value : undefined
  } catch {
    return undefined
  }
}

function isStreamEnvelope(value: unknown): value is StreamEnvelope {
  if (typeof value !== "object" || value === null) return false

  const envelope = value as Record<string, unknown>
  if (
    typeof envelope.run_id !== "string" ||
    typeof envelope.timestamp !== "string"
  )
    return false

  const event = envelope.event
  if (typeof event !== "object" || event === null) return false
  const data = (event as Record<string, unknown>).data
  if (typeof data !== "object" || data === null) return false

  return typeof (data as Record<string, unknown>).agent_id === "string"
}

function throwIfAborted(signal: AbortSignal): void {
  if (signal.aborted) throw new DOMException("recovery aborted", "AbortError")
}

function throwIfStreamCompleted(
  completion: StreamCompletion | undefined
): void {
  if (completion) throw completion
}

function isExpiredStreamCursor(error: unknown): boolean {
  return (
    isStreamCompletion(error) &&
    error.type === "stream_failed" &&
    error.error instanceof ApiError &&
    error.error.code === "cursor_expired"
  )
}

function connectionError(error: unknown): ApiError {
  const sourceError =
    isStreamCompletion(error) && error.type === "stream_failed"
      ? error.error
      : error
  if (sourceError instanceof ApiError && sourceError.code !== "cursor_expired")
    return sourceError

  return new ApiError(
    "connection_error",
    sourceError instanceof ApiError ? sourceError.status : 0,
    sourceError instanceof Error ? sourceError.message : "connection failed"
  )
}

function isStreamCompletion(value: unknown): value is StreamCompletion {
  return (
    typeof value === "object" &&
    value !== null &&
    ((value as { type?: unknown }).type === "stream_ended" ||
      (value as { type?: unknown }).type === "stream_failed")
  )
}
