import { useTranslation } from "react-i18next"

import {
  Message,
  MessageContent,
  MessageResponse,
} from "~/components/ai-elements/message"
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from "~/components/ai-elements/reasoning"
import {
  Tool,
  ToolContent,
  ToolHeader,
  type ToolStatus,
} from "~/components/ai-elements/tool"
import { CodeBlock } from "~/components/ai-elements/code-block"
import { MessageScrollerItem } from "~/components/ui/message-scroller"
import type {
  StableMessage,
  ToolProgress,
} from "~/features/agent-conversation/types"

type AgentMessageListProps = {
  messages: readonly StableMessage[]
  drafts: Readonly<Record<string, { text: string; reasoning: string }>>
  tools: Readonly<Record<string, ToolProgress>>
}

function toToolStatus(status: ToolProgress["status"]): ToolStatus {
  switch (status) {
    case "streaming":
      return "running"
    case "finished":
      return "completed"
    case "failed":
      return "error"
    default:
      return "pending"
  }
}

export function AgentMessageList({
  messages,
  drafts,
  tools,
}: AgentMessageListProps) {
  const { t, i18n } = useTranslation()
  const dateTimeFormat = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    dateStyle: "short",
    timeStyle: "short",
  })

  return (
    <>
      {messages.map((message) => {
        const isUser = message.role === "user"
        const text = message.text ?? JSON.stringify(message.json)

        return (
          <MessageScrollerItem
            key={`${message.agentId}:${message.businessSeq}`}
            messageId={`${message.agentId}:${message.businessSeq}`}
            scrollAnchor={isUser}
          >
            <Message from={isUser ? "user" : "assistant"}>
              <p className="text-xs text-muted-foreground">
                {isUser ? t("chat.you") : t("chat.assistant")}
              </p>
              {message.reasoning ? (
                <Reasoning>
                  <ReasoningTrigger
                    getThinkingMessage={(isStreaming) =>
                      isStreaming
                        ? t("chat.thinking")
                        : t("chat.reasoningComplete")
                    }
                  />
                  <ReasoningContent>{message.reasoning}</ReasoningContent>
                </Reasoning>
              ) : null}
              <MessageContent>
                <MessageResponse>{text}</MessageResponse>
              </MessageContent>
              <time
                dateTime={message.timestamp}
                className={
                  isUser
                    ? "self-end px-1 text-[0.625rem] text-muted-foreground"
                    : "px-1 text-[0.625rem] text-muted-foreground"
                }
              >
                {dateTimeFormat.format(new Date(message.timestamp))}
              </time>
            </Message>
          </MessageScrollerItem>
        )
      })}

      {Object.entries(drafts).map(([callId, draft]) => (
        <MessageScrollerItem key={callId} messageId={`draft:${callId}`}>
          <Message from="assistant">
            <p className="text-xs text-muted-foreground">
              {t("chat.assistant")} {t("chat.streamStatus")}
            </p>
            {draft.reasoning ? (
              <Reasoning isStreaming>
                <ReasoningTrigger
                  getThinkingMessage={(isStreaming) =>
                    isStreaming
                      ? t("chat.thinking")
                      : t("chat.reasoningComplete")
                  }
                />
                <ReasoningContent>{draft.reasoning}</ReasoningContent>
              </Reasoning>
            ) : null}
            <MessageContent>
              <MessageResponse>{draft.text}</MessageResponse>
            </MessageContent>
          </Message>
        </MessageScrollerItem>
      ))}

      {Object.values(tools).length > 0 ? (
        <MessageScrollerItem messageId="tool-process">
          <div className="flex flex-col gap-2">
            {Object.values(tools).map((tool) => (
              <Tool key={tool.callId} defaultOpen={tool.status === "streaming"}>
                <ToolHeader
                  status={toToolStatus(tool.status)}
                  statusLabel={t(`chat.toolStatus.${tool.status}`)}
                  title={tool.name ?? t("chat.unknownTool")}
                />
                <ToolContent>
                  {tool.argumentsText ? (
                    <CodeBlock code={tool.argumentsText} language="json" />
                  ) : null}
                  {tool.result ? (
                    <CodeBlock
                      code={JSON.stringify(tool.result, null, 2)}
                      language="json"
                    />
                  ) : null}
                  {tool.errorText ? <p>{tool.errorText}</p> : null}
                </ToolContent>
              </Tool>
            ))}
          </div>
        </MessageScrollerItem>
      ) : null}
    </>
  )
}
