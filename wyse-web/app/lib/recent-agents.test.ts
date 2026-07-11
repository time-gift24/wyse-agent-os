import { describe, expect, it } from "vitest"

import {
  clearCursor,
  createMemoryStorage,
  loadCursor,
  loadRecentAgents,
  rememberRecentAgent,
  removeRecentAgent,
  saveCursor,
} from "~/lib/recent-agents"

const recentAgent = (agentId: string, lastOpenedAt: string) => ({
  agentId,
  agentName: "coding-agent",
  title: `Request for ${agentId}`,
  lastOpenedAt,
})

describe("recent agents", () => {
  it("moves a reopened Agent to the front without duplicating it", () => {
    const storage = createMemoryStorage()
    rememberRecentAgent(storage, recentAgent("agent-1", "2026-07-11T00:00:00Z"))
    rememberRecentAgent(storage, recentAgent("agent-2", "2026-07-11T00:01:00Z"))
    rememberRecentAgent(storage, {
      ...recentAgent("agent-1", "2026-07-11T00:02:00Z"),
      title: "Reopened",
    })

    expect(loadRecentAgents(storage)).toEqual([
      {
        agentId: "agent-1",
        agentName: "coding-agent",
        title: "Reopened",
        lastOpenedAt: "2026-07-11T00:02:00Z",
      },
      recentAgent("agent-2", "2026-07-11T00:01:00Z"),
    ])
  })

  it("keeps only the twenty most recently opened Agents", () => {
    const storage = createMemoryStorage()

    for (let index = 0; index < 21; index++)
      rememberRecentAgent(storage, recentAgent(`agent-${index}`, `${index}`))

    expect(loadRecentAgents(storage).map((agent) => agent.agentId)).toEqual(
      Array.from({ length: 20 }, (_, index) => `agent-${20 - index}`)
    )
  })

  it("stores only Agent entry-point fields", () => {
    const storage = createMemoryStorage()
    const agent = {
      ...recentAgent("agent-1", "2026-07-11T00:00:00Z"),
      events: ["event content"],
      messages: ["message content"],
    }
    rememberRecentAgent(storage, agent)

    expect(storage.getItem("wyse-recent-agents")).toBe(
      JSON.stringify([recentAgent("agent-1", "2026-07-11T00:00:00Z")])
    )
  })

  it("removes malformed recent Agent data", () => {
    const storage = createMemoryStorage()
    storage.setItem("wyse-recent-agents", "not-json")

    expect(loadRecentAgents(storage)).toEqual([])
    expect(storage.getItem("wyse-recent-agents")).toBeNull()
  })

  it("removes one recent Agent while keeping the others", () => {
    const storage = createMemoryStorage()
    rememberRecentAgent(storage, recentAgent("agent-1", "2026-07-11T00:00:00Z"))
    rememberRecentAgent(storage, recentAgent("agent-2", "2026-07-11T00:01:00Z"))

    removeRecentAgent(storage, "agent-1")

    expect(loadRecentAgents(storage)).toEqual([
      recentAgent("agent-2", "2026-07-11T00:01:00Z"),
    ])
  })
})

describe("Agent cursors", () => {
  it("stores cursors separately from recent Agents and other cursor keys", () => {
    const storage = createMemoryStorage()
    rememberRecentAgent(storage, recentAgent("agent-1", "2026-07-11T00:00:00Z"))
    saveCursor(storage, "agent-1", "cursor-1")
    saveCursor(storage, "agent-2", "cursor-2")

    expect(loadCursor(storage, "agent-1")).toBe("cursor-1")
    expect(loadCursor(storage, "agent-2")).toBe("cursor-2")
    expect(loadRecentAgents(storage)).toEqual([
      recentAgent("agent-1", "2026-07-11T00:00:00Z"),
    ])
  })

  it("clears only the requested Agent cursor", () => {
    const storage = createMemoryStorage()
    saveCursor(storage, "agent-1", "cursor-1")
    saveCursor(storage, "agent-2", "cursor-2")

    clearCursor(storage, "agent-1")

    expect(loadCursor(storage, "agent-1")).toBeUndefined()
    expect(loadCursor(storage, "agent-2")).toBe("cursor-2")
  })
})
