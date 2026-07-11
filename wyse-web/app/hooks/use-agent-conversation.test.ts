import { beforeEach, describe, expect, it, vi } from "vitest"

const hooks = vi.hoisted(() => ({
  dispatch: vi.fn(),
  rememberRecentAgent: vi.fn(),
  createAgent: vi.fn(),
}))

vi.mock("react", () => ({
  useCallback: <T>(callback: T): T => callback,
  useEffect: () => {},
  useReducer: <T>(_: unknown, initial: T) => [initial, hooks.dispatch],
  useRef: <T>(initial: T) => ({ current: initial }),
  useState: <T>(initial: T) => [initial, () => {}],
}))

vi.mock("~/lib/wyse-api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("~/lib/wyse-api")>()
  return {
    ...actual,
    createWyseApi: () => ({ createAgent: hooks.createAgent }),
  }
})

vi.mock("~/lib/recent-agents", async (importOriginal) => {
  const actual = await importOriginal<typeof import("~/lib/recent-agents")>()
  return {
    ...actual,
    loadRecentAgents: () => [],
    rememberRecentAgent: hooks.rememberRecentAgent,
  }
})

import { useAgentConversation } from "~/hooks/use-agent-conversation"

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
})
