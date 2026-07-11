import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

const readyState = {
  agentId: null,
  view: null,
  messages: [],
  drafts: {},
  tools: {},
  approvals: {},
  phase: "empty" as const,
  error: null,
}

vi.mock("~/hooks/use-agent-conversation", () => ({
  useAgentConversation: () => ({
    state: readyState,
    recentAgents: [],
    selectAgent: vi.fn(),
    createConversation: vi.fn(),
    sendMessage: vi.fn(),
    resume: vi.fn(),
    cancel: vi.fn(),
    resolveApproval: vi.fn(),
    reconnect: vi.fn(),
    removeRecentAgent: vi.fn(),
  }),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

describe("ChatWorkspace", () => {
  it("keeps the established main conversation canvas and composer placement", async () => {
    const { ChatWorkspace } = await import("~/components/chat-workspace")
    const html = renderToStaticMarkup(<ChatWorkspace />)

    expect(html).toContain('data-slot="chat-main"')
    expect(html).toContain("h-[80dvh]")
    expect(html).toContain("min-h-[36rem]")
    expect(html.indexOf('data-slot="message-scroller"')).toBeLessThan(
      html.lastIndexOf('data-slot="card"')
    )
  })

  it("renders the live empty conversation state instead of static fixture messages", async () => {
    const { ChatWorkspace } = await import("~/components/chat-workspace")
    const html = renderToStaticMarkup(<ChatWorkspace />)

    expect(html).toContain("chat.empty")
    expect(html).not.toContain("chat.messages.assistantIntro")
  })
})
