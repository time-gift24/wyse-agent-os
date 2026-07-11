import type { AgentEvent, LlmEvent, StreamEnvelope } from "~/lib/wyse-api"
import type {
  ApprovalRequest,
  ConversationAction,
  ConversationState,
  StableMessage,
  ToolProgress,
} from "~/features/agent-conversation/types"

export const initialConversationState: ConversationState = {
  agentId: null,
  view: null,
  messages: [],
  drafts: {},
  tools: {},
  approvals: {},
  phase: "empty",
  error: null,
}

export function conversationReducer(
  state: ConversationState,
  action: ConversationAction
): ConversationState {
  switch (action.type) {
    case "agent_selected":
      return action.agentId === null
        ? initialConversationState
        : {
            ...initialConversationState,
            agentId: action.agentId,
            phase: "recovering",
          }
    case "recovery_started":
      return {
        ...initialConversationState,
        agentId: action.agentId,
        phase: "recovering",
      }
    case "view_loaded":
      if (state.agentId !== null && state.agentId !== action.view.agent_id)
        return state
      return { ...state, agentId: action.view.agent_id, view: action.view }
    case "history_loaded":
      return action.events.reduce(projectEnvelope, state)
    case "envelope_received":
      return projectEnvelope(state, action.envelope)
    case "recovery_ready":
      return { ...state, phase: "ready", error: null }
    case "connection_error":
      return { ...state, phase: "connection_error", error: action.error }
    case "missing":
      return { ...state, phase: "missing", error: action.error }
  }
}

function projectEnvelope(
  state: ConversationState,
  envelope: StreamEnvelope
): ConversationState {
  const { agent_id: agentId, event } = envelope.event.data
  if (state.agentId !== null && state.agentId !== agentId) return state

  return projectAgentEvent(state, agentId, envelope, event)
}

function projectAgentEvent(
  state: ConversationState,
  agentId: string,
  envelope: StreamEnvelope,
  event: AgentEvent
): ConversationState {
  switch (event.type) {
    case "message":
      return projectStableMessage(state, agentId, envelope)
    case "started":
      return updateViewStatus(state, "running")
    case "finished":
    case "failed":
    case "cancelled":
      return { ...updateViewStatus(state, "idle"), drafts: {} }
    case "tool_approval_requested": {
      const approval: ApprovalRequest = {
        approvalId: event.data.approval_id,
        agentName: event.data.agent_name,
        callId: event.data.call_id,
        toolName: event.data.tool_name,
        arguments: event.data.arguments,
        toolKind: event.data.tool_kind,
        dangerLevel: event.data.danger_level,
      }
      return {
        ...state,
        approvals: { ...state.approvals, [approval.approvalId]: approval },
      }
    }
    case "tool_approval_resolved": {
      if (!(event.data.approval_id in state.approvals)) return state
      const { [event.data.approval_id]: _, ...approvals } = state.approvals
      return { ...state, approvals }
    }
    case "llm":
      return projectLlmEvent(state, event.data.llm_call_id, event.data.event)
  }
}

function projectStableMessage(
  state: ConversationState,
  agentId: string,
  envelope: StreamEnvelope
): ConversationState {
  if (envelope.business_seq === undefined) return state

  const key = `${agentId}:${envelope.business_seq}`
  if (
    state.messages.some(
      (message) => `${message.agentId}:${message.businessSeq}` === key
    )
  ) {
    return state
  }

  const message = envelope.event.data.event
  if (message.type !== "message") return state

  const stableMessage: StableMessage = {
    agentId,
    businessSeq: envelope.business_seq,
    role: message.data.message.role,
    text:
      message.data.message.content.type === "text"
        ? message.data.message.content.data
        : null,
    json:
      message.data.message.content.type === "json"
        ? message.data.message.content.data
        : null,
    reasoning: message.data.message.reasoning_content ?? null,
    toolCalls: message.data.message.tool_calls.map((toolCall) => ({
      callId: toolCall.call_id,
      name: toolCall.name,
      arguments: toolCall.arguments,
    })),
    timestamp: envelope.timestamp,
  }

  return {
    ...state,
    messages: [...state.messages, stableMessage].sort(
      (left, right) => left.businessSeq - right.businessSeq
    ),
  }
}

function projectLlmEvent(
  state: ConversationState,
  llmCallId: string,
  event: LlmEvent
): ConversationState {
  switch (event.type) {
    case "text_delta":
      return event.data.role === "assistant"
        ? updateDraft(state, llmCallId, { text: event.data.delta })
        : state
    case "reasoning_delta":
      return updateDraft(state, llmCallId, { reasoning: event.data.delta })
    case "tool_call_started":
      return updateTool(state, event.data.call_id, {
        name: event.data.name,
        status: "streaming",
      })
    case "tool_call_delta":
      return updateTool(state, event.data.call_id, {
        name: event.data.name,
        argumentsText: event.data.arguments_delta,
        status: "streaming",
      })
    case "tool_call_finished":
      return updateTool(state, event.data.call_id, {
        result: event.data.result,
        errorText: null,
        status: "finished",
      })
    case "tool_call_failed":
      return updateTool(state, event.data.call_id, {
        errorText: event.data.error_text,
        status: "failed",
      })
    case "started":
    case "finished":
    case "failed":
      return state
  }
}

function updateDraft(
  state: ConversationState,
  llmCallId: string,
  delta: Partial<{ text: string; reasoning: string }>
): ConversationState {
  const draft = state.drafts[llmCallId] ?? { text: "", reasoning: "" }
  return {
    ...state,
    drafts: {
      ...state.drafts,
      [llmCallId]: {
        text: draft.text + (delta.text ?? ""),
        reasoning: draft.reasoning + (delta.reasoning ?? ""),
      },
    },
  }
}

function updateTool(
  state: ConversationState,
  callId: string,
  update: Partial<Omit<ToolProgress, "callId" | "argumentsText">> & {
    argumentsText?: string
  }
): ConversationState {
  const existing = state.tools[callId]
  const tool: ToolProgress = {
    callId,
    name: update.name ?? existing?.name ?? null,
    argumentsText:
      (existing?.argumentsText ?? "") + (update.argumentsText ?? ""),
    result: update.result ?? existing?.result ?? null,
    errorText: update.errorText ?? existing?.errorText ?? null,
    status: update.status ?? existing?.status ?? "streaming",
  }
  return { ...state, tools: { ...state.tools, [callId]: tool } }
}

function updateViewStatus(
  state: ConversationState,
  status: "idle" | "running"
): ConversationState {
  return state.view === null
    ? state
    : { ...state, view: { ...state.view, status } }
}
