import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

import { AgentApprovalCard } from "~/components/agent-approval-card"
import {
  finishApprovalSubmission,
  startApprovalSubmission,
} from "~/components/agent-approval-submissions"
import type { ApprovalRequest } from "~/features/agent-conversation/types"

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
  useTranslation: () => ({
    i18n: { resolvedLanguage: "en-US" },
    t: (key: string) => key,
  }),
}))

describe("ChatWorkspace", () => {
  it("keeps the established full-viewport conversation canvas and composer placement", async () => {
    const { ChatWorkspace } = await import("~/components/chat-workspace")
    const html = renderToStaticMarkup(<ChatWorkspace />)

    expect(html).toContain('data-slot="chat-main"')
    expect(html).toContain('id="longzhong" class="h-[100dvh]')
    expect(html).toContain("2xl:h-[100dvh]")
    expect(html).not.toContain("min-h-[36rem]")
    expect(html).not.toContain("scroll-mt-20")
    expect(html.indexOf('data-slot="message-scroller"')).toBeLessThan(
      html.lastIndexOf('data-slot="card"')
    )
  })

  it("renders a blank live conversation rather than invented onboarding copy", async () => {
    const { ChatWorkspace } = await import("~/components/chat-workspace")
    const html = renderToStaticMarkup(<ChatWorkspace />)

    expect(html).not.toContain("chat.empty")
    expect(html).not.toContain("chat.startConversation")
    expect(html).not.toContain(">WYSE<")
    expect(html).not.toContain("chat.messages.assistantIntro")
  })

  it("uses the composed AI Elements prompt shell without moving the chat canvas", async () => {
    const { ChatWorkspace } = await import("~/components/chat-workspace")
    const html = renderToStaticMarkup(<ChatWorkspace />)

    expect(html).toContain('data-slot="prompt-input"')
    expect(html).toContain('data-slot="prompt-input-body"')
    expect(html).toContain('data-slot="prompt-input-tools"')
    expect(html.indexOf('data-slot="chat-main"')).toBeLessThan(
      html.indexOf('data-slot="prompt-input"')
    )
  })

  it("keeps each approval card disabled until its own decision settles", async () => {
    const first = deferred<void>()
    const second = deferred<void>()
    let submitting = new Set<string>()

    const decide = async (approvalId: string, decision: Promise<void>) => {
      submitting = startApprovalSubmission(submitting, approvalId)
      try {
        await decision
      } finally {
        submitting = finishApprovalSubmission(submitting, approvalId)
      }
    }

    const firstDecision = decide("approval-1", first.promise)
    const secondDecision = decide("approval-2", second.promise)

    expect(renderApprovals(submitting)).toHaveLength(4)

    first.resolve()
    await firstDecision
    expect(renderApprovals(submitting)).toHaveLength(2)

    second.resolve()
    await secondDecision
    expect(renderApprovals(submitting)).toHaveLength(0)
  })
})

const deferred = <T,>() => {
  let resolve: (value: T) => void = () => {}
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise
  })
  return { promise, resolve }
}

const renderApprovals = (submitting: ReadonlySet<string>) => {
  const html = renderToStaticMarkup(
    <>
      <AgentApprovalCard
        approval={approval("approval-1", "first-tool")}
        submitting={submitting.has("approval-1")}
        onDecision={() => {}}
      />
      <AgentApprovalCard
        approval={approval("approval-2", "second-tool")}
        submitting={submitting.has("approval-2")}
        onDecision={() => {}}
      />
    </>
  )

  return html.match(/<button[^>]*disabled=""[^>]*>/g) ?? []
}

const approval = (approvalId: string, toolName: string): ApprovalRequest => ({
  approvalId,
  agentName: "coding-agent",
  callId: `call-${approvalId}`,
  toolName,
  arguments: {},
  toolKind: "write",
  dangerLevel: "high",
})
