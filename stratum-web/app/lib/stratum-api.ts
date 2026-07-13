import type {
  AgentTemplateView,
  ModelConfig,
  ModelDescriptor,
} from "./model-config"

export class ApiError extends Error {
  constructor(
    readonly code: string,
    readonly status: number,
    message: string
  ) {
    super(message)
    this.name = "ApiError"
  }
}

export type AgentView = {
  agent_id: string
  agent_name: string
  status: "idle" | "running"
  model_config: ModelConfig
  run_id: string | null
  turn_id: string | null
  last_seq: number
  updated_at: string
}

export type ToolCall = { call_id: string; name: string; arguments: unknown }
export type ChatMessage = {
  role: "user" | "assistant" | "tool" | "system"
  content: { type: "text"; data: string } | { type: "json"; data: unknown }
  tool_calls?: readonly ToolCall[]
  reasoning_content?: string
  tool_call_id?: string
}
export type LlmEvent =
  | {
      type: "text_delta"
      data: { role: "assistant" | "user" | "tool" | "system"; delta: string }
    }
  | { type: "reasoning_delta"; data: { delta: string } }
  | {
      type: "tool_call_started"
      data: { call_id: string; name: string | null }
    }
  | {
      type: "tool_call_delta"
      data: { call_id: string; name: string | null; arguments_delta: string }
    }
  | { type: "tool_call_finished"; data: { call_id: string; result: unknown } }
  | { type: "tool_call_failed"; data: { call_id: string; error_text: string } }
  | { type: "started" }
  | { type: "finished"; data: { finish_reason: string; usage: unknown } }
  | { type: "failed"; data: { error_text: string } }
export type AgentEvent =
  | { type: "message"; data: { turn_id: string; message: ChatMessage } }
  | { type: "started"; data: { turn_id: string } }
  | { type: "finished"; data: { finish_reason: string; usage: unknown } }
  | { type: "failed"; data: { error_text: string; usage: unknown } }
  | { type: "cancelled"; data: { usage: unknown } }
  | {
      type: "tool_approval_requested"
      data: {
        approval_id: string
        agent_name: string
        call_id: string
        tool_name: string
        arguments: unknown
        tool_kind: "read" | "write"
        danger_level: "low" | "medium" | "high"
      }
    }
  | {
      type: "tool_approval_resolved"
      data: { approval_id: string; decision: "approve" | "reject" }
    }
  | { type: "llm"; data: { llm_call_id: string; event: LlmEvent } }

export type StreamEnvelope = {
  business_seq?: number
  run_id: string
  timestamp: string
  event: {
    type: "agent"
    data: { agent_id: string; event: AgentEvent }
  }
}

export type HistoryPage = {
  through_seq: number
  events: readonly StreamEnvelope[]
  next_front_seq: number
  has_more: boolean
}

export type StratumApi = {
  createAgent(input: {
    agentName: string
    text: string
    modelConfig?: ModelConfig
  }): Promise<{ agent_id: string; agent_name: string; run_id: string }>
  getAgentTemplates(): Promise<readonly AgentTemplateView[]>
  getModels(): Promise<readonly ModelDescriptor[]>
  getAgent(agentId: string): Promise<AgentView>
  getHistory(
    agentId: string,
    query: { afterSeq: number; throughSeq: number; limit: number }
  ): Promise<HistoryPage>
  sendMessage(
    agentId: string,
    text: string,
    modelConfig?: ModelConfig
  ): Promise<void>
  resume(agentId: string): Promise<void>
  cancel(agentId: string): Promise<void>
  resolveApproval(
    agentId: string,
    approvalId: string,
    decision: "approve" | "reject"
  ): Promise<void>
}

type ApiErrorBody = { error?: { code?: unknown; message?: unknown } }

const isApiErrorBody = (value: unknown): value is ApiErrorBody =>
  typeof value === "object" && value !== null

export async function apiErrorFromResponse(
  response: Response
): Promise<ApiError> {
  try {
    const body: unknown = await response.json()
    if (
      isApiErrorBody(body) &&
      typeof body.error?.code === "string" &&
      typeof body.error.message === "string"
    ) {
      return new ApiError(body.error.code, response.status, body.error.message)
    }
  } catch {
    // Invalid or missing error JSON intentionally receives the safe fallback.
  }

  return new ApiError("http_error", response.status, "request failed")
}

export function createStratumApi(options: {
  baseUrl: string
  fetcher?: typeof fetch
}): StratumApi {
  const baseUrl = options.baseUrl.replace(/\/$/, "")
  const fetcher = options.fetcher ?? fetch

  const request = async <T>(path: string, init?: RequestInit): Promise<T> => {
    const response = await fetcher(`${baseUrl}${path}`, init)
    if (!response.ok) throw await apiErrorFromResponse(response)
    return response.json() as Promise<T>
  }

  const command = async (path: string, body?: unknown): Promise<void> => {
    const response = await fetcher(`${baseUrl}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    })
    if (!response.ok) throw await apiErrorFromResponse(response)
  }

  return {
    createAgent: (input) =>
      request("/v1/agents", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          agent_name: input.agentName,
          text: input.text,
          ...(input.modelConfig === undefined
            ? {}
            : { model_config: input.modelConfig }),
        }),
      }),
    getAgentTemplates: async () => {
      const response = await request<{ agents: readonly AgentTemplateView[] }>(
        "/v1/agent/templates"
      )
      return response.agents
    },
    getModels: async () => {
      const response = await request<{ models: readonly ModelDescriptor[] }>(
        "/v1/models"
      )
      return response.models
    },
    getAgent: (agentId) => request(`/v1/agents/${agentId}`),
    getHistory: (agentId, query) => {
      const search = new URLSearchParams({
        after_seq: String(query.afterSeq),
        through_seq: String(query.throughSeq),
        limit: String(query.limit),
      })
      return request(`/v1/agents/${agentId}/messages?${search}`)
    },
    sendMessage: (agentId, text, modelConfig) =>
      command(`/v1/agents/${agentId}/messages`, {
        text,
        ...(modelConfig === undefined ? {} : { model_config: modelConfig }),
      }),
    resume: (agentId) => command(`/v1/agents/${agentId}/resume`),
    cancel: (agentId) => command(`/v1/agents/${agentId}/cancel`),
    resolveApproval: (agentId, approvalId, decision) =>
      command(`/v1/agents/${agentId}/approvals/${approvalId}`, { decision }),
  }
}
