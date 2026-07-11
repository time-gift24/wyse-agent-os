import { describe, expect, it, vi } from "vitest"

import type { ConversationAction } from "~/features/agent-conversation/types"
import {
  recoverConversation,
  type RecoveryDependencies,
} from "~/features/agent-conversation/recovery"
import {
  ApiError,
  type AgentView,
  type HistoryPage,
  type StreamEnvelope,
} from "~/lib/wyse-api"
import type { SseEvent } from "~/lib/wyse-event-stream"

const agentView = (overrides: Partial<AgentView> = {}): AgentView => ({
  agent_id: "agent-1",
  agent_name: "coding-agent",
  status: "idle",
  run_id: null,
  turn_id: null,
  last_seq: 0,
  updated_at: "2026-07-11T00:00:00Z",
  ...overrides,
})

const messageEnvelope = (
  agentId: string,
  businessSeq: number,
  text: string
): StreamEnvelope => ({
  business_seq: businessSeq,
  run_id: "run-1",
  timestamp: "2026-07-11T00:00:00Z",
  event: {
    type: "agent",
    data: {
      agent_id: agentId,
      event: {
        type: "message",
        data: {
          turn_id: "turn-1",
          message: {
            role: "assistant",
            content: { type: "text", data: text },
            tool_calls: [],
          },
        },
      },
    },
  },
})

const sseEnvelope = (id: string, envelope: StreamEnvelope): SseEvent => ({
  id,
  event: "message",
  data: JSON.stringify(envelope),
})

const historyPage = (
  events: readonly StreamEnvelope[],
  throughSeq: number,
  hasMore: boolean
): HistoryPage => ({
  events: [...events],
  through_seq: throughSeq,
  next_front_seq: throughSeq,
  has_more: hasMore,
})

const createRecoveryHarness = (overrides: Partial<RecoveryDependencies>) => {
  const controller = new AbortController()
  const dependencies: RecoveryDependencies = {
    api: {
      getAgent: async () => agentView(),
      getHistory: async () => historyPage([], 0, false),
    },
    subscribe: () => ({ done: new Promise(() => {}) }),
    loadCursor: () => undefined,
    saveCursor: () => {},
    clearCursor: () => {},
    dispatch: () => {},
    ...overrides,
  }
  return {
    recover: (agentId: string) =>
      recoverConversation(dependencies, { agentId, signal: controller.signal }),
  }
}

describe("recoverConversation", () => {
  it("starts the stream before reading AgentView and drains buffered events after fixed history", async () => {
    const calls: string[] = []
    const dispatched: ConversationAction[] = []
    const recovery = createRecoveryHarness({
      subscribe: ({ onEvent }) => {
        calls.push("subscribe")
        onEvent(sseEnvelope("11", messageEnvelope("agent-1", 3, "live")))
        return { done: new Promise(() => {}) }
      },
      api: {
        getAgent: async () => {
          calls.push("view")
          return agentView({ last_seq: 2 })
        },
        getHistory: async () => {
          calls.push("history")
          return historyPage(
            [messageEnvelope("agent-1", 2, "stored")],
            2,
            false
          )
        },
      },
      dispatch: (action) => dispatched.push(action),
    })

    await recovery.recover("agent-1")

    expect(calls).toEqual(["subscribe", "view", "history"])
    expect(dispatched.map((action) => action.type)).toEqual([
      "recovery_started",
      "view_loaded",
      "history_loaded",
      "envelope_received",
      "recovery_ready",
    ])
  })

  it("clears only the expired Agent cursor then restarts with replay=all", async () => {
    const afterCursors: Array<string | undefined> = []
    const clearCursor = vi.fn()
    let attempts = 0
    const recovery = createRecoveryHarness({
      subscribe: ({ afterCursor }: { afterCursor?: string }) => {
        afterCursors.push(afterCursor)
        attempts += 1
        return {
          done:
            attempts === 1
              ? Promise.reject(
                  new ApiError("cursor_expired", 410, "event cursor expired")
                )
              : new Promise(() => {}),
        }
      },
      loadCursor: () => "99",
      clearCursor,
      api: {
        getAgent: async () => agentView({ last_seq: 0 }),
        getHistory: async () => historyPage([], 0, false),
      },
      dispatch: vi.fn(),
    })

    await recovery.recover("agent-1")

    expect(clearCursor).toHaveBeenCalledWith("agent-1")
    expect(afterCursors).toEqual(["99", undefined])
  })

  it("does not mark recovery ready when the stream closes during history loading", async () => {
    const dispatched: ConversationAction[] = []
    const recovery = createRecoveryHarness({
      subscribe: () => ({ done: Promise.resolve() }),
      dispatch: (action) => dispatched.push(action),
    })

    await recovery.recover("agent-1")

    expect(dispatched.map((action) => action.type)).toEqual([
      "recovery_started",
      "connection_error",
    ])
  })

  it("transitions to a connection error after a ready stream closes", async () => {
    let closeStream: () => void = () => {}
    const dispatched: ConversationAction[] = []
    const recovery = createRecoveryHarness({
      subscribe: () => ({
        done: new Promise<void>((resolve) => {
          closeStream = resolve
        }),
      }),
      dispatch: (action) => dispatched.push(action),
    })

    await recovery.recover("agent-1")
    closeStream()
    await Promise.resolve()

    expect(dispatched.map((action) => action.type)).toEqual([
      "recovery_started",
      "view_loaded",
      "history_loaded",
      "recovery_ready",
      "connection_error",
    ])
  })

  it.each(["getAgent", "getHistory"] as const)(
    "does not retry a cursor_expired error from %s",
    async (failingRequest) => {
      const clearCursor = vi.fn()
      const subscribe = vi.fn(() => ({ done: new Promise<void>(() => {}) }))
      const expired = new ApiError("cursor_expired", 410, "not an SSE error")
      const recovery = createRecoveryHarness({
        subscribe,
        clearCursor,
        api: {
          getAgent: async () => {
            if (failingRequest === "getAgent") throw expired
            return agentView()
          },
          getHistory: async () => {
            if (failingRequest === "getHistory") throw expired
            return historyPage([], 0, false)
          },
        },
      })

      await recovery.recover("agent-1")

      expect(subscribe).toHaveBeenCalledOnce()
      expect(clearCursor).not.toHaveBeenCalled()
    }
  )

  it("persists a cursor only after accepting an envelope for the selected Agent", async () => {
    const dispatched: ConversationAction[] = []
    const saveCursor = vi.fn()
    const recovery = createRecoveryHarness({
      subscribe: ({ onEvent }) => {
        onEvent(sseEnvelope("10", messageEnvelope("agent-2", 1, "other")))
        onEvent(sseEnvelope("11", messageEnvelope("agent-1", 1, "selected")))
        return { done: new Promise(() => {}) }
      },
      saveCursor,
      dispatch: (action) => dispatched.push(action),
    })

    await recovery.recover("agent-1")

    expect(dispatched).toContainEqual({
      type: "envelope_received",
      envelope: messageEnvelope("agent-1", 1, "selected"),
    })
    expect(saveCursor).toHaveBeenCalledExactlyOnceWith("agent-1", "11")
  })
})
