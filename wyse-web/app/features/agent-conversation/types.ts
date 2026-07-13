import type { AgentView, ApiError, StreamEnvelope } from "~/lib/wyse-api"

export type StableMessage = {
  agentId: string
  businessSeq: number
  role: "user" | "assistant" | "tool" | "system"
  text: string | null
  json: unknown | null
  reasoning: string | null
  toolCalls: readonly { callId: string; name: string; arguments: unknown }[]
  timestamp: string
}

export type ToolProgress = {
  callId: string
  llmCallId: string
  name: string | null
  argumentsText: string
  result: unknown | null
  errorText: string | null
  status: "streaming" | "finished" | "failed"
}

export type ApprovalRequest = {
  approvalId: string
  agentName: string
  callId: string
  toolName: string
  arguments: unknown
  toolKind: "read" | "write"
  dangerLevel: "low" | "medium" | "high"
}

export type ConversationState = {
  agentId: string | null
  view: AgentView | null
  messages: readonly StableMessage[]
  drafts: Readonly<Record<string, { text: string; reasoning: string }>>
  tools: Readonly<Record<string, ToolProgress>>
  approvals: Readonly<Record<string, ApprovalRequest>>
  phase: "empty" | "recovering" | "ready" | "connection_error" | "missing"
  error: ApiError | null
}

export type ConversationAction =
  | { type: "agent_selected"; agentId: string | null }
  | { type: "recovery_started"; agentId: string; preserveTransient?: boolean }
  | { type: "view_loaded"; view: AgentView }
  | { type: "history_loaded"; events: readonly StreamEnvelope[] }
  | { type: "envelope_received"; envelope: StreamEnvelope }
  | { type: "recovery_ready" }
  | { type: "connection_error"; error: ApiError }
  | { type: "missing"; error: ApiError | null }
