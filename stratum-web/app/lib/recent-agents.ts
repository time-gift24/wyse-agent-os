const RECENT_AGENTS_KEY = "stratum-recent-agents"
const MAX_RECENT_AGENTS = 20

export type RecentAgent = {
  agentId: string
  agentName: string
  title: string
  lastOpenedAt: string
}

export type StorageLike = Pick<Storage, "getItem" | "setItem" | "removeItem">

export const createMemoryStorage = (): StorageLike => {
  const values = new Map<string, string>()

  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  }
}

export const loadRecentAgents = (storage: StorageLike): RecentAgent[] => {
  let stored: string | null

  try {
    stored = storage.getItem(RECENT_AGENTS_KEY)
  } catch {
    return []
  }

  if (stored === null) return []

  try {
    const agents: unknown = JSON.parse(stored)
    if (Array.isArray(agents) && agents.every(isRecentAgent)) {
      const recentAgents = agents.slice(0, MAX_RECENT_AGENTS).map(toRecentAgent)
      saveRecentAgents(storage, recentAgents)
      return recentAgents
    }
  } catch {
    // Remove corrupt data below.
  }

  try {
    storage.removeItem(RECENT_AGENTS_KEY)
  } catch {
    // Storage can be unavailable in private browsing or when disabled.
  }

  return []
}

export const rememberRecentAgent = (
  storage: StorageLike,
  agent: RecentAgent
): void => {
  saveRecentAgents(storage, [
    agent,
    ...loadRecentAgents(storage).filter(
      (recentAgent) => recentAgent.agentId !== agent.agentId
    ),
  ])
}

export const removeRecentAgent = (
  storage: StorageLike,
  agentId: string
): void => {
  const agents = loadRecentAgents(storage)
  const remainingAgents = agents.filter((agent) => agent.agentId !== agentId)

  if (remainingAgents.length !== agents.length)
    saveRecentAgents(storage, remainingAgents)
}

export const loadCursor = (
  storage: StorageLike,
  agentId: string
): string | undefined => {
  try {
    return storage.getItem(cursorKey(agentId)) ?? undefined
  } catch {
    return undefined
  }
}

export const saveCursor = (
  storage: StorageLike,
  agentId: string,
  cursor: string
): void => {
  try {
    storage.setItem(cursorKey(agentId), cursor)
  } catch {
    // Storage can be unavailable in private browsing or when disabled.
  }
}

export const clearCursor = (storage: StorageLike, agentId: string): void => {
  try {
    storage.removeItem(cursorKey(agentId))
  } catch {
    // Storage can be unavailable in private browsing or when disabled.
  }
}

const saveRecentAgents = (
  storage: StorageLike,
  agents: readonly RecentAgent[]
): void => {
  try {
    storage.setItem(
      RECENT_AGENTS_KEY,
      JSON.stringify(agents.slice(0, MAX_RECENT_AGENTS).map(toRecentAgent))
    )
  } catch {
    // Storage can be unavailable in private browsing or when disabled.
  }
}

const cursorKey = (agentId: string): string => `stratum-agent-cursor:${agentId}`

const isRecentAgent = (value: unknown): value is RecentAgent => {
  if (typeof value !== "object" || value === null) return false

  const agent = value as Record<string, unknown>
  return (
    typeof agent.agentId === "string" &&
    typeof agent.agentName === "string" &&
    typeof agent.title === "string" &&
    typeof agent.lastOpenedAt === "string"
  )
}

const toRecentAgent = (agent: RecentAgent): RecentAgent => ({
  agentId: agent.agentId,
  agentName: agent.agentName,
  title: agent.title,
  lastOpenedAt: agent.lastOpenedAt,
})
