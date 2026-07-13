import { renderToStaticMarkup } from "react-dom/server"
import { I18nextProvider } from "react-i18next"
import { describe, expect, it, vi } from "vitest"

import { createI18n } from "~/lib/i18n"
import { ChatWorkspace } from "./chat-workspace"

const conversationState = vi.hoisted(() => ({
  agentId: null as string | null,
}))

vi.mock("use-stick-to-bottom", () => ({
  useStickToBottom: () => ({
    scrollRef: () => undefined,
    contentRef: () => undefined,
    scrollToBottom: () => undefined,
    isAtBottom: true,
  }),
}))

vi.mock("~/hooks/use-agent-conversation", () => ({
  useAgentConversation: () => ({
    state: {
      agentId: conversationState.agentId,
      view: null,
      messages: [],
      drafts: {},
      tools: {},
      approvals: {},
      phase: "empty",
      error: null,
    },
    recentAgents: [],
    composerConfiguration: {
      agentTemplates: [],
      models: [],
      metadataLoading: false,
      metadataError: null,
      selectedTemplate: null,
      agentName: "default-agent",
      persistedModelConfig: null,
      currentModelConfig: null,
      selectedModelConfig: null,
      existingAgent: false,
      turnRunning: false,
      selectTemplate: () => undefined,
      selectModel: () => undefined,
      setThinkingLevel: () => undefined,
    },
    selectAgent: () => undefined,
    removeRecentAgent: () => undefined,
    createConversation: async () => false,
    sendMessage: async () => false,
    resolveApproval: async () => undefined,
    reconnect: () => undefined,
    cancel: async () => undefined,
  }),
}))

const i18n = createI18n("en")

function renderWorkspace() {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <ChatWorkspace />
    </I18nextProvider>
  )
}

describe("chat workspace composer", () => {
  it("renders a single centered 116px composer surface for a new conversation", () => {
    conversationState.agentId = null

    const html = renderWorkspace()

    expect(html).toContain('data-slot="prompt-input"')
    expect(html).not.toContain('data-slot="card"')
    expect(html).toContain('data-slot="chat-composer-positioner"')
    expect(html).toContain('data-composer-position="centered"')
    expect(html).toContain("min-h-[7.25rem]")
    expect(html).toContain("min-h-[3.875rem]")
    expect(html).toContain("rounded-[0.75rem]")
    expect(html).toContain("size-11")
  })

  it("docks the same composer one rem above the viewport for an existing conversation", () => {
    conversationState.agentId = "agent-1"

    const html = renderWorkspace()

    expect(html).toContain('data-composer-position="docked"')
    expect(html).toContain("bottom:1rem")
    expect(html).toContain("translate-y-0")
  })
})
