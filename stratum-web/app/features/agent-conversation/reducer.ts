import {
  ApiError,
  type AgentEvent,
  type LlmEvent,
  type StreamEnvelope,
} from "~/lib/stratum-api"
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
      if (action.preserveTransient && state.agentId === action.agentId)
        return { ...state, phase: "recovering", error: null }
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
      return { ...state, phase: "ready" }
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
      return { ...updateViewStatus(state, "running"), error: null }
    case "finished":
    case "cancelled":
      return {
        ...updateViewStatus(state, "idle"),
        drafts: {},
        approvals: {},
        error: null,
      }
    case "failed":
      return {
        ...updateViewStatus(state, "idle"),
        drafts: {},
        approvals: {},
        error: new ApiError("agent_failed", 500, event.data.error_text),
      }
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

  const event = envelope.event.data.event
  if (event.type !== "message") return state

  const { message, turn_id: turnId } = event.data
  if (message.role === "tool") {
    return projectPersistedToolResult(state, turnId, message)
  }

  const key = `${agentId}:${envelope.business_seq}`
  if (
    state.messages.some(
      (message) => `${message.agentId}:${message.businessSeq}` === key
    )
  ) {
    return state
  }

  const stableMessage: StableMessage = {
    agentId,
    businessSeq: envelope.business_seq,
    role: message.role,
    text: message.content.type === "text" ? message.content.data : null,
    json: message.content.type === "json" ? message.content.data : null,
    reasoning: message.reasoning_content ?? null,
    toolCalls: (message.tool_calls ?? []).map((toolCall) => ({
      callId: toolCall.call_id,
      name: toolCall.name,
      arguments: toolCall.arguments,
    })),
    timestamp: envelope.timestamp,
  }

  const stateWithTools = stableMessage.toolCalls.reduce(
    (currentState, toolCall) =>
      projectPersistedToolCall(currentState, turnId, toolCall),
    state
  )

  return {
    ...stateWithTools,
    messages: [...state.messages, stableMessage].sort(
      (left, right) => left.businessSeq - right.businessSeq
    ),
  }
}

function projectPersistedToolCall(
  state: ConversationState,
  turnId: string,
  toolCall: StableMessage["toolCalls"][number]
): ConversationState {
  const existing = state.tools[toolCall.callId]
  const tool: ToolProgress = {
    callId: toolCall.callId,
    llmCallId: existing?.llmCallId || `turn:${turnId}`,
    name: existing?.name ?? toolCall.name,
    argumentsText:
      existing?.argumentsText || JSON.stringify(toolCall.arguments),
    result: existing?.result ?? null,
    errorText: existing?.errorText ?? null,
    status: existing?.status ?? "streaming",
  }
  return { ...state, tools: { ...state.tools, [tool.callId]: tool } }
}

function projectPersistedToolResult(
  state: ConversationState,
  turnId: string,
  message: Extract<AgentEvent, { type: "message" }>["data"]["message"]
): ConversationState {
  if (!message.tool_call_id) return state

  const existing = state.tools[message.tool_call_id]
  const result = message.content.data
  const tool: ToolProgress = {
    callId: message.tool_call_id,
    llmCallId: existing?.llmCallId || `turn:${turnId}`,
    name: existing?.name ?? null,
    argumentsText: existing?.argumentsText ?? "",
    result,
    errorText: null,
    status: "finished",
  }
  return { ...state, tools: { ...state.tools, [tool.callId]: tool } }
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
        llmCallId,
        name: event.data.name,
        status: "streaming",
      })
    case "tool_call_delta":
      return updateTool(state, event.data.call_id, {
        llmCallId,
        name: event.data.name,
        argumentsText: event.data.arguments_delta,
        status: "streaming",
      })
    case "tool_call_finished":
      return updateTool(state, event.data.call_id, {
        llmCallId,
        result: event.data.result,
        errorText: null,
        status: "finished",
      })
    case "tool_call_failed":
      return updateTool(state, event.data.call_id, {
        llmCallId,
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
    llmCallId: update.llmCallId ?? existing?.llmCallId ?? "",
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
