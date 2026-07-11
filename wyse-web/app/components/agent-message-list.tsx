import { useTranslation } from "react-i18next"

import { StratumMark } from "~/components/stratum-mark"
import { Bubble, BubbleContent } from "~/components/ui/bubble"
import {
  Message,
  MessageAvatar,
  MessageContent,
  MessageFooter,
  MessageHeader,
} from "~/components/ui/message"
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
  const { t } = useTranslation()

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
            <Message align={isUser ? "end" : "start"}>
              {isUser ? null : (
                <MessageAvatar>
                  <StratumMark
                    animated={false}
                    variant="compact"
                    className="size-6"
                  />
                </MessageAvatar>
              )}
              <MessageContent>
                <MessageHeader>
                  {isUser ? t("chat.you") : t("chat.assistant")}
                </MessageHeader>
                <Bubble
                  variant={isUser ? "secondary" : "ghost"}
                  align={isUser ? "end" : "start"}
                >
                  <BubbleContent>{text}</BubbleContent>
                </Bubble>
                {message.reasoning ? (
                  <details className="text-muted-foreground">
                    <summary>{t("chat.reasoning")}</summary>
                    <p className="mt-1 whitespace-pre-wrap">
                      {message.reasoning}
                    </p>
                  </details>
                ) : null}
                <MessageFooter>{message.timestamp}</MessageFooter>
              </MessageContent>
            </Message>
          </MessageScrollerItem>
        )
      })}

      {Object.entries(drafts).map(([callId, draft]) => (
        <MessageScrollerItem key={callId} messageId={`draft:${callId}`}>
          <Message align="start">
            <MessageAvatar>
              <StratumMark
                animated={false}
                variant="compact"
                className="size-6"
              />
            </MessageAvatar>
            <MessageContent>
              <MessageHeader>
                {t("chat.assistant")} · {t("chat.streamStatus")}
              </MessageHeader>
              <Bubble variant="ghost" align="start">
                <BubbleContent>
                  <span aria-label={t("chat.streamStatus")} className="mr-1">
                    ●
                  </span>
                  {draft.text}
                </BubbleContent>
              </Bubble>
              {draft.reasoning ? (
                <details className="text-muted-foreground">
                  <summary>{t("chat.reasoning")}</summary>
                  <p className="mt-1 whitespace-pre-wrap">{draft.reasoning}</p>
                </details>
              ) : null}
            </MessageContent>
          </Message>
        </MessageScrollerItem>
      ))}

      {Object.values(tools).length > 0 ? (
        <MessageScrollerItem messageId="tool-process">
          <details className="text-xs/relaxed text-muted-foreground">
            <summary>{t("chat.toolProcess")}</summary>
            <div className="mt-1 flex flex-col gap-1">
              {Object.values(tools).map((tool) => (
                <p key={tool.callId}>
                  {tool.name ?? t("chat.unknownTool")} · {tool.status}
                  {tool.errorText ? ` · ${tool.errorText}` : ""}
                </p>
              ))}
            </div>
          </details>
        </MessageScrollerItem>
      ) : null}
    </>
  )
}
