import { beforeEach, describe, expect, it, vi } from "vitest"

const hooks = vi.hoisted(() => ({
  dispatch: vi.fn(),
  rememberRecentAgent: vi.fn(),
  createAgent: vi.fn(),
  sendMessage: vi.fn(),
  state: undefined as unknown,
  recentAgents: [] as Array<{
    agentId: string
    agentName: string
    title: string
    lastOpenedAt: string
  }>,
}))

vi.mock("react", () => ({
  useCallback: <T>(callback: T): T => callback,
  useEffect: () => {},
  useReducer: <T>(_: unknown, initial: T) => [
    (hooks.state ?? initial) as T,
    hooks.dispatch,
  ],
  useRef: <T>(initial: T) => ({ current: initial }),
  useState: <T>(initial: T) => [initial, () => {}],
}))

vi.mock("~/lib/wyse-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("~/lib/wyse-api")>()
  return {
    ...actual,
    createWyseApi: () => ({
      createAgent: hooks.createAgent,
      sendMessage: hooks.sendMessage,
    }),
  }
})

vi.mock("~/lib/recent-agents", async (importOriginal) => {
  const actual = await importOriginal<typeof import("~/lib/recent-agents")>()
  return {
    ...actual,
    loadRecentAgents: () => hooks.recentAgents,
    rememberRecentAgent: hooks.rememberRecentAgent,
  }
})

import { useAgentConversation } from "~/hooks/use-agent-conversation"
import { ApiError } from "~/lib/wyse-api"

const deferred = <T>() => {
  let resolve: (value: T) => void = () => {}
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

describe("useAgentConversation", () => {
  beforeEach(() => {
    hooks.dispatch.mockReset()
    hooks.rememberRecentAgent.mockReset()
    hooks.createAgent.mockReset()
    hooks.sendMessage.mockReset()
    hooks.state = undefined
    hooks.recentAgents = []
    vi.stubEnv("VITE_WYSE_API_BASE_URL", "https://api.example.test")
    vi.stubEnv("VITE_DEFAULT_AGENT_NAME", "coding-agent")
    vi.stubGlobal("window", {
      localStorage: {
        getItem: () => null,
        setItem: () => {},
        removeItem: () => {},
      },
    })
  })

  it("ignores a created Agent after the selection changes while createAgent is pending", async () => {
    const created = deferred<{
      agent_id: string
      agent_name: string
      run_id: string
    }>()
    hooks.createAgent.mockReturnValue(created.promise)
    const conversation = useAgentConversation()

    const creating = conversation.createConversation("start a conversation")
    conversation.selectAgent("agent-existing")
    created.resolve({
      agent_id: "agent-created",
      agent_name: "coding-agent",
      run_id: "run-1",
    })
    await creating

    expect(hooks.rememberRecentAgent).not.toHaveBeenCalled()
    expect(hooks.dispatch).toHaveBeenCalledWith({
      type: "agent_selected",
      agentId: "agent-existing",
    })
    expect(hooks.dispatch).not.toHaveBeenCalledWith({
      type: "agent_selected",
      agentId: "agent-created",
    })
  })

  it("does not send while the selected Agent is recovering or running", async () => {
    hooks.state = {
      agentId: "agent-1",
      view: {
        agent_id: "agent-1",
        agent_name: "coding-agent",
        status: "running",
        run_id: "run-1",
        turn_id: "turn-1",
        last_seq: 0,
        updated_at: "2026-07-11T00:00:00Z",
      },
      messages: [],
      drafts: {},
      tools: {},
      approvals: {},
      phase: "ready",
      error: null,
    }
    const conversation = useAgentConversation()
    conversation.selectAgent("agent-1")

    await expect(conversation.sendMessage("next")).resolves.toBe(false)
    expect(hooks.sendMessage).not.toHaveBeenCalled()
    expect(hooks.dispatch).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "connection_error" })
    )
  })

  it("treats agent_busy as an unsuccessful command without changing to a connection error", async () => {
    hooks.sendMessage.mockRejectedValue(
      new ApiError("agent_busy", 409, "agent is busy")
    )
    const conversation = useAgentConversation()
    conversation.selectAgent("agent-1")

    await expect(conversation.sendMessage("next")).resolves.toBe(false)
    expect(hooks.dispatch).not.toHaveBeenCalledWith(
      expect.objectContaining({ type: "connection_error" })
    )
  })

  it("refreshes an existing recent Agent when it is reopened", () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date("2026-07-11T00:02:00Z"))
    hooks.recentAgents = [
      {
        agentId: "agent-1",
        agentName: "coding-agent",
        title: "Earlier chat",
        lastOpenedAt: "2026-07-11T00:00:00Z",
      },
    ]

    useAgentConversation().selectAgent("agent-1")

    expect(hooks.rememberRecentAgent).toHaveBeenCalledWith(expect.anything(), {
      agentId: "agent-1",
      agentName: "coding-agent",
      title: "Earlier chat",
      lastOpenedAt: "2026-07-11T00:02:00.000Z",
    })
    vi.useRealTimers()
  })
})
