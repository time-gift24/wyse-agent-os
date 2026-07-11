import { useCallback, useEffect, useReducer, useRef, useState } from "react"

import {
  initialConversationState,
  conversationReducer,
} from "~/features/agent-conversation/reducer"
import { recoverConversation } from "~/features/agent-conversation/recovery"
import type { ConversationState } from "~/features/agent-conversation/types"
import {
  clearCursor,
  loadCursor,
  loadRecentAgents,
  rememberRecentAgent,
  removeRecentAgent as removeStoredRecentAgent,
  saveCursor,
  type RecentAgent,
  type StorageLike,
} from "~/lib/recent-agents"
import { createWyseApi, ApiError } from "~/lib/wyse-api"
import { subscribeToAgentEvents } from "~/lib/wyse-event-stream"

export type AgentConversation = {
  state: ConversationState
  recentAgents: readonly RecentAgent[]
  selectAgent(agentId: string | null): void
  createConversation(text: string): Promise<boolean>
  sendMessage(text: string): Promise<boolean>
  resume(): Promise<void>
  cancel(): Promise<void>
  resolveApproval(
    approvalId: string,
    decision: "approve" | "reject"
  ): Promise<void>
  reconnect(): void
  removeRecentAgent(agentId: string): void
}

export function useAgentConversation(): AgentConversation {
  const [state, dispatch] = useReducer(
    conversationReducer,
    initialConversationState
  )
  const [recentAgents, setRecentAgents] = useState<readonly RecentAgent[]>([])
  const [selectedAgentId, setSelectedAgentId] = useState<string | null>(null)
  const [reconnectVersion, setReconnectVersion] = useState(0)
  const selectedAgentRef = useRef<string | null>(null)
  const selectionGeneration = useRef(0)

  useEffect(() => {
    const storage = browserStorage()
    if (storage) setRecentAgents(loadRecentAgents(storage))
  }, [])

  const refreshRecentAgent = useCallback((agentId: string) => {
    const lastOpenedAt = new Date().toISOString()
    const storage = browserStorage()

    if (storage) {
      const existing = loadRecentAgents(storage).find(
        (agent) => agent.agentId === agentId
      )
      if (!existing) return

      rememberRecentAgent(storage, { ...existing, lastOpenedAt })
      setRecentAgents(loadRecentAgents(storage))
      return
    }

    setRecentAgents((agents) => {
      const existing = agents.find((agent) => agent.agentId === agentId)
      return existing
        ? [
            { ...existing, lastOpenedAt },
            ...agents.filter((agent) => agent.agentId !== agentId),
          ]
        : agents
    })
  }, [])

  const selectAgent = useCallback(
    (agentId: string | null) => {
      if (agentId !== null) refreshRecentAgent(agentId)
      selectionGeneration.current += 1
      selectedAgentRef.current = agentId
      setSelectedAgentId(agentId)
      dispatch({ type: "agent_selected", agentId })
    },
    [refreshRecentAgent]
  )

  useEffect(() => {
    if (selectedAgentId === null) return

    const controller = new AbortController()
    const generation = selectionGeneration.current
    const configuration = apiConfiguration()
    if (configuration instanceof ApiError) {
      if (generation === selectionGeneration.current)
        dispatch({ type: "connection_error", error: configuration })
      return () => controller.abort()
    }

    const storage = browserStorage()
    const api = createWyseApi({ baseUrl: configuration.baseUrl })
    const dispatchIfCurrent = (action: Parameters<typeof dispatch>[0]) => {
      if (
        !controller.signal.aborted &&
        generation === selectionGeneration.current
      )
        dispatch(action)
    }

    void recoverConversation(
      {
        api,
        subscribe: (options) =>
          subscribeToAgentEvents({
            ...options,
            baseUrl: configuration.baseUrl,
          }),
        loadCursor: (agentId) =>
          storage ? loadCursor(storage, agentId) : undefined,
        saveCursor: (agentId, cursor) => {
          if (
            storage &&
            !controller.signal.aborted &&
            generation === selectionGeneration.current
          )
            saveCursor(storage, agentId, cursor)
        },
        clearCursor: (agentId) => {
          if (storage && generation === selectionGeneration.current)
            clearCursor(storage, agentId)
        },
        dispatch: dispatchIfCurrent,
      },
      { agentId: selectedAgentId, signal: controller.signal }
    )

    return () => controller.abort()
  }, [reconnectVersion, selectedAgentId])

  const reportError = useCallback((error: unknown) => {
    const apiError = toApiError(error)
    dispatch(
      apiError.status === 404
        ? { type: "missing", error: apiError }
        : { type: "connection_error", error: apiError }
    )
  }, [])

  const reconnect = useCallback(() => {
    if (selectedAgentRef.current === null) return
    selectionGeneration.current += 1
    setReconnectVersion((version) => version + 1)
  }, [])

  const createConversation = useCallback(
    async (text: string) => {
      const prompt = text.trim()
      if (prompt === "") {
        reportError(new ApiError("invalid_input", 400, "message is required"))
        return false
      }

      const configuration = apiConfiguration()
      if (configuration instanceof ApiError) {
        reportError(configuration)
        return false
      }

      const generation = selectionGeneration.current
      try {
        const created = await createWyseApi({
          baseUrl: configuration.baseUrl,
        }).createAgent({ agentName: configuration.agentName, text: prompt })
        if (generation !== selectionGeneration.current) return false

        const recentAgent: RecentAgent = {
          agentId: created.agent_id,
          agentName: created.agent_name,
          title: prompt,
          lastOpenedAt: new Date().toISOString(),
        }
        const storage = browserStorage()
        if (storage) {
          rememberRecentAgent(storage, recentAgent)
          setRecentAgents(loadRecentAgents(storage))
        } else {
          setRecentAgents((agents) => [
            recentAgent,
            ...agents.filter((agent) => agent.agentId !== recentAgent.agentId),
          ])
        }
        selectAgent(created.agent_id)
        return true
      } catch (error) {
        if (generation === selectionGeneration.current) reportError(error)
        return false
      }
    },
    [reportError, selectAgent]
  )

  const selectedClient = useCallback(() => {
    const agentId = selectedAgentRef.current
    if (agentId === null) {
      reportError(
        new ApiError("agent_not_selected", 400, "select an agent first")
      )
      return undefined
    }

    const configuration = apiConfiguration()
    if (configuration instanceof ApiError) {
      reportError(configuration)
      return undefined
    }

    return {
      api: createWyseApi({ baseUrl: configuration.baseUrl }),
      agentId,
      generation: selectionGeneration.current,
    }
  }, [reportError])

  const sendMessage = useCallback(
    async (text: string) => {
      const message = text.trim()
      if (message === "") {
        reportError(new ApiError("invalid_input", 400, "message is required"))
        return false
      }

      if (state.phase === "recovering" || state.view?.status === "running")
        return false

      const client = selectedClient()
      if (!client) return false

      try {
        await client.api.sendMessage(client.agentId, message)
        return true
      } catch (error) {
        if (
          client.generation === selectionGeneration.current &&
          !isApiErrorCode(error, "agent_busy")
        )
          reportError(error)
        return false
      }
    },
    [reportError, selectedClient, state.phase, state.view?.status]
  )

  const resume = useCallback(async () => {
    const client = selectedClient()
    if (!client) return

    try {
      await client.api.resume(client.agentId)
    } catch (error) {
      if (client.generation !== selectionGeneration.current) return
      if (isApiErrorCode(error, "resume_not_running")) {
        reconnect()
        return
      }
      reportError(error)
    }
  }, [reconnect, reportError, selectedClient])

  const cancel = useCallback(async () => {
    const client = selectedClient()
    if (!client) return

    try {
      await client.api.cancel(client.agentId)
    } catch (error) {
      if (client.generation !== selectionGeneration.current) return
      if (isApiErrorCode(error, "resume_required")) {
        reconnect()
        return
      }
      reportError(error)
    }
  }, [reconnect, reportError, selectedClient])

  const resolveApproval = useCallback(
    async (approvalId: string, decision: "approve" | "reject") => {
      const client = selectedClient()
      if (!client) return

      try {
        await client.api.resolveApproval(client.agentId, approvalId, decision)
      } catch (error) {
        if (client.generation === selectionGeneration.current)
          reportError(error)
      }
    },
    [reportError, selectedClient]
  )

  const removeRecentAgent = useCallback((agentId: string) => {
    const storage = browserStorage()
    if (storage) {
      removeStoredRecentAgent(storage, agentId)
      setRecentAgents(loadRecentAgents(storage))
      return
    }
    setRecentAgents((agents) =>
      agents.filter((agent) => agent.agentId !== agentId)
    )
  }, [])

  return {
    state,
    recentAgents,
    selectAgent,
    createConversation,
    sendMessage,
    resume,
    cancel,
    resolveApproval,
    reconnect,
    removeRecentAgent,
  }
}

function apiConfiguration(): { baseUrl: string; agentName: string } | ApiError {
  const baseUrl = import.meta.env.VITE_WYSE_API_BASE_URL?.trim()
  const agentName = import.meta.env.VITE_DEFAULT_AGENT_NAME?.trim()
  if (baseUrl && agentName) return { baseUrl, agentName }

  return new ApiError(
    "configuration_missing",
    0,
    "VITE_WYSE_API_BASE_URL and VITE_DEFAULT_AGENT_NAME are required"
  )
}

function browserStorage(): StorageLike | undefined {
  if (typeof window === "undefined") return undefined

  try {
    return window.localStorage
  } catch {
    return undefined
  }
}

function isApiErrorCode(error: unknown, code: string): boolean {
  return error instanceof ApiError && error.code === code
}

function toApiError(error: unknown): ApiError {
  if (error instanceof ApiError) return error
  return new ApiError(
    "command_failed",
    0,
    error instanceof Error ? error.message : "command failed"
  )
}
