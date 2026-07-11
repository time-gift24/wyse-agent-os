import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    i18n: { resolvedLanguage: "zh-CN" },
    t: (key: string) =>
      ({
        "chat.assistant": "隆中对",
        "chat.reasoningComplete": "推理完成",
        "chat.thinking": "正在思考",
        "chat.toolStatus.streaming": "正在执行",
        "chat.streamStatus": "流式输出中",
      })[key] ?? key,
  }),
}))

import { AgentMessageList } from "~/components/agent-message-list"
import {
  MessageScroller,
  MessageScrollerProvider,
} from "~/components/ui/message-scroller"

describe("AgentMessageList", () => {
  it("places localized completed reasoning before its assistant response", () => {
    const html = renderToStaticMarkup(
      <MessageScrollerProvider>
        <MessageScroller>
          <AgentMessageList
            messages={[
              {
                agentId: "agent-1",
                businessSeq: 2,
                role: "assistant",
                text: "Mock response",
                json: null,
                reasoning: "Checking the request…",
                toolCalls: [],
                timestamp: "2026-07-12T00:00:00Z",
              },
            ]}
            drafts={{}}
            tools={{}}
          />
        </MessageScroller>
      </MessageScrollerProvider>
    )

    expect(html).toContain("推理完成")
    expect(html.indexOf("推理完成")).toBeLessThan(html.indexOf("Mock response"))
    expect(html).toContain('dateTime="2026-07-12T00:00:00Z"')
    expect(html).not.toContain(">2026-07-12T00:00:00Z</time>")
  })

  it("localizes tool progress instead of exposing wire protocol statuses", () => {
    const html = renderToStaticMarkup(
      <MessageScrollerProvider>
        <MessageScroller>
          <AgentMessageList
            messages={[]}
            drafts={{}}
            tools={{
              "call-1": {
                callId: "call-1",
                name: "read_file",
                argumentsText: "",
                result: null,
                errorText: null,
                status: "streaming",
              },
            }}
          />
        </MessageScroller>
      </MessageScrollerProvider>
    )

    expect(html).toContain("正在执行")
    expect(html).not.toContain(">streaming<")
  })

  it("places localized streaming reasoning before partial assistant text", () => {
    const html = renderToStaticMarkup(
      <MessageScrollerProvider>
        <MessageScroller>
          <AgentMessageList
            messages={[]}
            drafts={{ "llm-1": { text: "partial", reasoning: "analyzing" } }}
            tools={{}}
          />
        </MessageScroller>
      </MessageScrollerProvider>
    )

    expect(html).toContain('aria-label="流式输出中"')
    expect(html.indexOf("正在思考")).toBeLessThan(html.indexOf("partial"))
  })
})
