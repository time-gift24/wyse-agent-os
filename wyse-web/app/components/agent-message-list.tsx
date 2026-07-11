import { useTranslation } from "react-i18next"

import {
  AiMessage,
  AiMessageContent,
  AiMessageHeader,
  AiStreamingMark,
} from "~/components/ai-elements/message"
import { AiReasoning } from "~/components/ai-elements/reasoning"
import { AiTool } from "~/components/ai-elements/tool"
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
          <AiMessage from={isUser ? "user" : "assistant"}>
            <AiMessageHeader>
              {isUser ? t("chat.you") : t("chat.assistant")}
            </AiMessageHeader>
            {message.reasoning ? (
              <AiReasoning
                completeLabel={t("chat.reasoningComplete")}
                thinkingLabel={t("chat.thinking")}
              >
                {message.reasoning}
              </AiReasoning>
            ) : null}
            <AiMessageContent from={isUser ? "user" : "assistant"}>
              {text}
            </AiMessageContent>
            <time
              dateTime={message.timestamp}
              className="px-1 text-[0.625rem] text-muted-foreground"
            >
              {dateTimeFormat.format(new Date(message.timestamp))}
            </time>
          </AiMessage>
          </MessageScrollerItem>
        )
      })}

      {Object.entries(drafts).map(([callId, draft]) => (
        <MessageScrollerItem key={callId} messageId={`draft:${callId}`}>
          <AiMessage from="assistant">
            <AiMessageHeader>
              {t("chat.assistant")} {t("chat.streamStatus")}
            </AiMessageHeader>
            {draft.reasoning ? (
              <AiReasoning
                streaming
                completeLabel={t("chat.reasoningComplete")}
                thinkingLabel={t("chat.thinking")}
              >
                {draft.reasoning}
              </AiReasoning>
            ) : null}
            <AiMessageContent from="assistant">
              <AiStreamingMark label={t("chat.streamStatus")} />
              {draft.text}
            </AiMessageContent>
          </AiMessage>
        </MessageScrollerItem>
      ))}

      {Object.values(tools).length > 0 ? (
        <MessageScrollerItem messageId="tool-process">
          <div className="flex flex-col gap-2">
            {Object.values(tools).map((tool) => (
              <AiTool
                key={tool.callId}
                name={tool.name ?? t("chat.unknownTool")}
                status={t(`chat.toolStatus.${tool.status}`)}
              >
                {tool.argumentsText ? <pre>{tool.argumentsText}</pre> : null}
                {tool.result ? (
                  <pre>{JSON.stringify(tool.result, null, 2)}</pre>
                ) : null}
                {tool.errorText ? <p>{tool.errorText}</p> : null}
              </AiTool>
            ))}
          </div>
        </MessageScrollerItem>
      ) : null}
    </>
  )
}
