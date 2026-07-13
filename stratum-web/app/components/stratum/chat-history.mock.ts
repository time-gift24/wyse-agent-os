import type { RecentAgent } from "~/lib/recent-agents"

export function getMockRecentAgents(t: (key: string) => string): RecentAgent[] {
  const now = Date.now()
  const assistantName = t("chat.assistant")

  return [
    {
      agentId: "mock-vite-config",
      agentName: assistantName,
      title: t("chat.history.mock.viteConfig"),
      lastOpenedAt: new Date(now - 2 * 60 * 60 * 1000).toISOString(),
    },
    {
      agentId: "mock-retry-policy",
      agentName: assistantName,
      title: t("chat.history.mock.retryPolicy"),
      lastOpenedAt: new Date(now - 24 * 60 * 60 * 1000).toISOString(),
    },
    {
      agentId: "mock-agent-state",
      agentName: assistantName,
      title: t("chat.history.mock.agentTypes"),
      lastOpenedAt: new Date(now - 3 * 24 * 60 * 60 * 1000).toISOString(),
    },
  ]
}
