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
import {
  configForModel,
  configForTemplate,
  withThinkingLevel,
  type AgentTemplateView,
  type ModelConfig,
  type ModelDescriptor,
} from "~/lib/model-config"
import { createStratumApi, ApiError } from "~/lib/stratum-api"
import { subscribeToAgentEvents } from "~/lib/stratum-event-stream"

const STRATUM_API_BASE_URL = "http://127.0.0.1:18080"

export type ComposerConfiguration = {
  agentTemplates: readonly AgentTemplateView[]
  models: readonly ModelDescriptor[]
  metadataLoading: boolean
  metadataError: ApiError | null
  selectedTemplate: AgentTemplateView | null
  agentName: string | null
  persistedModelConfig: ModelConfig | null
  currentModelConfig: ModelConfig | null
  selectedModelConfig: ModelConfig | null
  existingAgent: boolean
  turnRunning: boolean
  selectTemplate(template: AgentTemplateView): void
  selectModel(descriptor: ModelDescriptor): void
  setThinkingLevel(level: "disabled" | "high" | "max"): void
}

export type AgentConversation = {
  state: ConversationState
  recentAgents: readonly RecentAgent[]
  composerConfiguration: ComposerConfiguration
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
  const [agentTemplates, setAgentTemplates] = useState<
    readonly AgentTemplateView[]
  >([])
  const [models, setModels] = useState<readonly ModelDescriptor[]>([])
  const [metadataLoading, setMetadataLoading] = useState(true)
  const [metadataError, setMetadataError] = useState<ApiError | null>(null)
  const [selectedTemplate, setSelectedTemplate] =
    useState<AgentTemplateView | null>(null)
  const [requestedModelConfig, setRequestedModelConfig] =
    useState<ModelConfig | null>(null)
  const [acceptedModelConfig, setAcceptedModelConfig] =
    useState<ModelConfig | null>(null)
  const selectedAgentRef = useRef<string | null>(null)
  const selectionGeneration = useRef(0)

  useEffect(() => {
    const storage = browserStorage()
    if (storage) setRecentAgents(loadRecentAgents(storage))
  }, [])

  useEffect(() => {
    let active = true
    const api = createStratumApi({ baseUrl: STRATUM_API_BASE_URL })
    void Promise.all([api.getAgentTemplates(), api.getModels()]).then(
      ([templates, descriptors]) => {
        if (!active) return
        setAgentTemplates(templates)
        setModels(descriptors)
        setMetadataError(null)
        setMetadataLoading(false)
      },
      (error: unknown) => {
        if (!active) return
        setMetadataError(toApiError(error))
        setMetadataLoading(false)
      }
    )

    return () => {
      active = false
    }
  }, [])

  const selectAgent = useCallback((agentId: string | null) => {
    selectionGeneration.current += 1
    selectedAgentRef.current = agentId
    if (agentId === null) setSelectedTemplate(null)
    setSelectedAgentId(agentId)
    dispatch({ type: "agent_selected", agentId })
  }, [])

  useEffect(() => {
    if (state.agentId !== null || selectedTemplate !== null) return

    const defaultTemplate = agentTemplates[0]
    if (defaultTemplate !== undefined) setSelectedTemplate(defaultTemplate)
  }, [agentTemplates, selectedTemplate, state.agentId])

  useEffect(() => {
    setRequestedModelConfig(null)
    setAcceptedModelConfig(null)
  }, [state.agentId])

  useEffect(() => {
    if (selectedAgentId === null) return

    const controller = new AbortController()
    const generation = selectionGeneration.current
    const storage = browserStorage()
    const api = createStratumApi({ baseUrl: STRATUM_API_BASE_URL })
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
            baseUrl: STRATUM_API_BASE_URL,
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

      if (selectedTemplate === null) {
        reportError(
          new ApiError(
            "agent_template_not_selected",
            400,
            "select an agent first"
          )
        )
        return false
      }

      const generation = selectionGeneration.current
      try {
        const created = await createStratumApi({
          baseUrl: STRATUM_API_BASE_URL,
        }).createAgent({
          agentName: selectedTemplate.agent_name,
          text: prompt,
          modelConfig: requestedModelConfig ?? undefined,
        })
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
    [reportError, requestedModelConfig, selectAgent, selectedTemplate]
  )

  const selectedClient = useCallback(() => {
    const agentId = selectedAgentRef.current
    if (agentId === null) {
      reportError(
        new ApiError("agent_not_selected", 400, "select an agent first")
      )
      return undefined
    }

    return {
      api: createStratumApi({ baseUrl: STRATUM_API_BASE_URL }),
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

      const selectedConfig = requestedModelConfig
      try {
        await client.api.sendMessage(
          client.agentId,
          message,
          selectedConfig ?? undefined
        )
        if (
          selectedConfig !== null &&
          client.generation === selectionGeneration.current
        ) {
          setAcceptedModelConfig(selectedConfig)
          setRequestedModelConfig((pendingConfig) =>
            pendingConfig === selectedConfig ? null : pendingConfig
          )
        }
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
    [
      reportError,
      requestedModelConfig,
      selectedClient,
      state.phase,
      state.view?.status,
    ]
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

  const selectTemplate = useCallback(
    (template: AgentTemplateView) => {
      if (selectedAgentRef.current !== null) selectAgent(null)
      setRequestedModelConfig(null)
      setAcceptedModelConfig(null)
      setSelectedTemplate(template)
    },
    [selectAgent]
  )

  const persistedModelConfig = state.view?.model_config ?? null
  const currentModelConfig =
    state.agentId === null
      ? selectedTemplate === null
        ? null
        : configForTemplate(selectedTemplate)
      : (acceptedModelConfig ?? persistedModelConfig)
  const selectedModelConfig = requestedModelConfig ?? currentModelConfig

  const selectModel = useCallback((descriptor: ModelDescriptor) => {
    setRequestedModelConfig(configForModel(descriptor))
  }, [])

  const setThinkingLevel = useCallback(
    (level: "disabled" | "high" | "max") => {
      if (selectedModelConfig === null) return
      setRequestedModelConfig({
        ...selectedModelConfig,
        parameters: withThinkingLevel(selectedModelConfig.parameters, level),
      })
    },
    [selectedModelConfig]
  )

  const composerConfiguration: ComposerConfiguration = {
    agentTemplates,
    models,
    metadataLoading,
    metadataError,
    selectedTemplate,
    agentName:
      state.agentId === null
        ? (selectedTemplate?.agent_name ?? null)
        : (state.view?.agent_name ?? null),
    persistedModelConfig,
    currentModelConfig,
    selectedModelConfig,
    existingAgent: state.agentId !== null,
    turnRunning: state.view?.status === "running",
    selectTemplate,
    selectModel,
    setThinkingLevel,
  }

  return {
    state,
    recentAgents,
    composerConfiguration,
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
