"use client"

import { Clock3Icon, PlusIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "~/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "~/components/ui/card"
import type { ConversationState } from "~/features/agent-conversation/types"
import type { RecentAgent } from "~/lib/recent-agents"

type ChatHistoryProps = {
  state: ConversationState
  recentAgents: readonly RecentAgent[]
  onSelectAgent(agentId: string): void
  onRemoveAgent(agentId: string): void
  onNewConversation(): void
}

export function ChatHistory({
  state,
  recentAgents,
  onSelectAgent,
  onRemoveAgent,
  onNewConversation,
}: ChatHistoryProps) {
  const { t } = useTranslation()

  return (
    <Card
      data-slot="chat-history"
      size="sm"
      className="wyse-history-rail relative shrink-0 bg-transparent ring-0"
    >
      <CardHeader className="grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
        <CardTitle className="truncate">{t("chat.history.title")}</CardTitle>
        <CardAction>
          <Button
            variant="outline"
            size="icon-sm"
            aria-label={t("chat.history.new")}
            title={t("chat.history.new")}
            onClick={onNewConversation}
          >
            <PlusIcon aria-hidden="true" />
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-1.5 pt-0">
        {recentAgents.map((agent) => {
          const isCurrent = agent.agentId === state.agentId
          const isMissing = state.phase === "missing" && isCurrent

          return (
            <div key={agent.agentId} className="flex items-center gap-1">
              <Button
                variant={isCurrent ? "secondary" : "ghost"}
                size="lg"
                className="h-auto min-w-0 flex-1 justify-start py-2 text-left"
                onClick={() => onSelectAgent(agent.agentId)}
              >
                <span className="flex min-w-0 flex-1 flex-col items-start gap-0.5">
                  <span className="w-full truncate">{agent.title}</span>
                  <span className="flex items-center gap-1 text-[0.625rem] text-muted-foreground">
                    <Clock3Icon aria-hidden="true" />
                    {agent.lastOpenedAt}
                  </span>
                </span>
              </Button>
              {isMissing ? (
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => onRemoveAgent(agent.agentId)}
                >
                  {t("chat.removeLocalEntry")}
                </Button>
              ) : null}
            </div>
          )
        })}
      </CardContent>
    </Card>
  )
}
