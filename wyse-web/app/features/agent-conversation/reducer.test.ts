import { describe, expect, it } from "vitest"
import type {
  AgentEvent,
  AgentView,
  LlmEvent,
  StreamEnvelope,
} from "~/lib/wyse-api"
import {
  conversationReducer,
  initialConversationState,
} from "~/features/agent-conversation/reducer"
import type {
  ConversationAction,
  ConversationState,
} from "~/features/agent-conversation/types"

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

const agentEnvelope = (agentId: string, event: AgentEvent): StreamEnvelope => ({
  run_id: "run-1",
  timestamp: "2026-07-11T00:00:00Z",
  event: { type: "agent", data: { agent_id: agentId, event } },
})

const llmEnvelope = (
  agentId: string,
  llmCallId: string,
  event: LlmEvent
): StreamEnvelope =>
  agentEnvelope(agentId, {
    type: "llm",
    data: { llm_call_id: llmCallId, event },
  })

const agentView = (overrides: Partial<AgentView> = {}): AgentView => ({
  agent_id: "agent-1",
  agent_name: "coding-agent",
  status: "running",
  run_id: "run-1",
  turn_id: "turn-1",
  last_seq: 7,
  updated_at: "2026-07-11T00:00:00Z",
  ...overrides,
})

const reduceAll = (
  state: ConversationState,
  actions: readonly ConversationAction[]
): ConversationState => actions.reduce(conversationReducer, state)

describe("conversationReducer", () => {
  it("keeps one stable message when history and SSE replay share a business sequence", () => {
    const state = reduceAll(initialConversationState, [
      {
        type: "history_loaded",
        events: [messageEnvelope("agent-1", 7, "history")],
      },
      {
        type: "envelope_received",
        envelope: messageEnvelope("agent-1", 7, "replay"),
      },
    ])

    expect(state.messages).toHaveLength(1)
    expect(state.messages[0]).toMatchObject({ businessSeq: 7, text: "history" })
  })

  it("projects a persisted message that omits empty tool calls", () => {
    const envelope = {
      business_seq: 7,
      run_id: "run-1",
      timestamp: "2026-07-11T00:00:00Z",
      event: {
        type: "agent",
        data: {
          agent_id: "agent-1",
          event: {
            type: "message",
            data: {
              turn_id: "turn-1",
              message: {
                role: "user",
                content: { type: "text", data: "persisted" },
              },
            },
          },
        },
      },
    } as unknown as StreamEnvelope

    const state = conversationReducer(initialConversationState, {
      type: "envelope_received",
      envelope,
    })

    expect(state.messages).toMatchObject([
      { role: "user", text: "persisted", toolCalls: [] },
    ])
  })

  it("accumulates assistant text and reasoning by LLM call without changing stable history", () => {
    const state = reduceAll(initialConversationState, [
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "text_delta",
          data: { role: "assistant", delta: "hel" },
        }),
      },
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "reasoning_delta",
          data: { delta: "plan" },
        }),
      },
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "text_delta",
          data: { role: "tool", delta: "ignored" },
        }),
      },
    ])

    expect(state.messages).toHaveLength(0)
    expect(state.drafts["llm-1"]).toEqual({ text: "hel", reasoning: "plan" })
  })

  it("orders stable messages by ascending business sequence", () => {
    const state = reduceAll(initialConversationState, [
      {
        type: "history_loaded",
        events: [
          messageEnvelope("agent-1", 9, "later"),
          messageEnvelope("agent-1", 3, "earlier"),
        ],
      },
    ])

    expect(state.messages.map((message) => message.businessSeq)).toEqual([3, 9])
  })

  it("tracks streamed tool call progress through its finished result", () => {
    const state = reduceAll(initialConversationState, [
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "tool_call_started",
          data: { call_id: "call-1", name: "read_file" },
        }),
      },
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "tool_call_delta",
          data: {
            call_id: "call-1",
            name: null,
            arguments_delta: '{"path":"/tmp"}',
          },
        }),
      },
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "tool_call_finished",
          data: { call_id: "call-1", result: { contents: "ok" } },
        }),
      },
    ])

    expect(state.tools["call-1"]).toEqual({
      callId: "call-1",
      name: "read_file",
      argumentsText: '{"path":"/tmp"}',
      result: { contents: "ok" },
      errorText: null,
      status: "finished",
    })
  })

  it("adds an approval request and removes only its matching resolution", () => {
    const state = reduceAll(initialConversationState, [
      {
        type: "envelope_received",
        envelope: agentEnvelope("agent-1", {
          type: "tool_approval_requested",
          data: {
            approval_id: "approval-1",
            agent_name: "coding-agent",
            call_id: "call-1",
            tool_name: "write_file",
            arguments: { path: "/tmp/output" },
            tool_kind: "write",
            danger_level: "high",
          },
        }),
      },
      {
        type: "envelope_received",
        envelope: agentEnvelope("agent-1", {
          type: "tool_approval_requested",
          data: {
            approval_id: "approval-2",
            agent_name: "coding-agent",
            call_id: "call-2",
            tool_name: "read_file",
            arguments: { path: "/tmp/input" },
            tool_kind: "read",
            danger_level: "low",
          },
        }),
      },
      {
        type: "envelope_received",
        envelope: agentEnvelope("agent-1", {
          type: "tool_approval_resolved",
          data: { approval_id: "approval-1", decision: "approve" },
        }),
      },
    ])

    expect(state.approvals).not.toHaveProperty("approval-1")
    expect(state.approvals["approval-2"]).toMatchObject({
      approvalId: "approval-2",
      toolName: "read_file",
    })
  })

  it("marks a finished Agent idle and clears LLM drafts", () => {
    const state = reduceAll(initialConversationState, [
      { type: "recovery_started", agentId: "agent-1" },
      { type: "view_loaded", view: agentView() },
      {
        type: "envelope_received",
        envelope: llmEnvelope("agent-1", "llm-1", {
          type: "text_delta",
          data: { role: "assistant", delta: "working" },
        }),
      },
      {
        type: "envelope_received",
        envelope: agentEnvelope("agent-1", {
          type: "finished",
          data: { finish_reason: "stop", usage: null },
        }),
      },
    ])

    expect(state.view?.status).toBe("idle")
    expect(state.drafts).toEqual({})
  })

  it.each(["finished", "failed", "cancelled"] as const)(
    "clears pending approvals when an Agent is %s",
    (terminalEvent) => {
      const state = reduceAll(initialConversationState, [
        { type: "recovery_started", agentId: "agent-1" },
        { type: "view_loaded", view: agentView() },
        {
          type: "envelope_received",
          envelope: agentEnvelope("agent-1", {
            type: "tool_approval_requested",
            data: {
              approval_id: "approval-1",
              agent_name: "coding-agent",
              call_id: "call-1",
              tool_name: "read_file",
              arguments: {},
              tool_kind: "read",
              danger_level: "low",
            },
          }),
        },
        {
          type: "envelope_received",
          envelope: agentEnvelope(
            "agent-1",
            terminalEvent === "cancelled"
              ? { type: "cancelled", data: { usage: null } }
              : terminalEvent === "finished"
                ? {
                    type: "finished",
                    data: { finish_reason: "stop", usage: null },
                  }
                : {
                    type: "failed",
                    data: { error_text: "failed", usage: null },
                  }
          ),
        },
      ])

      expect(state.view?.status).toBe("idle")
      expect(state.drafts).toEqual({})
      expect(state.approvals).toEqual({})
    }
  )

  it("ignores an envelope for an Agent other than the selected Agent", () => {
    const state = reduceAll(initialConversationState, [
      { type: "recovery_started", agentId: "agent-1" },
      {
        type: "envelope_received",
        envelope: messageEnvelope("agent-2", 1, "other conversation"),
      },
    ])

    expect(state.messages).toEqual([])
  })
})
