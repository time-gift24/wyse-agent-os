"use client"

import { useEffect, useMemo } from "react"
import {
  Clock3Icon,
  HistoryIcon,
  PlusIcon,
  Trash2Icon,
  XIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { AnimatePresence, motion, useReducedMotion } from "motion/react"

import { AnimatedList } from "~/components/react-bits/AnimatedList"
import { Button } from "~/components/ui/button"
import type { ConversationState } from "~/features/agent-conversation/types"
import type { RecentAgent } from "~/lib/recent-agents"
import { cn } from "~/lib/utils"

import { getMockRecentAgents } from "./chat-history.mock"

type ChatHistoryProps = {
  open: boolean
  onClose(): void
  state: ConversationState
  recentAgents: readonly RecentAgent[]
  onSelectAgent(agentId: string): void
  onRemoveAgent(agentId: string): void
  onNewConversation(): void
}

function formatRelativeTime(iso: string, locale: string): string {
  try {
    const date = new Date(iso)
    const now = new Date()
    const seconds = Math.floor((now.getTime() - date.getTime()) / 1000)
    const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" })

    if (seconds < 60) return rtf.format(-seconds, "second")
    const minutes = Math.floor(seconds / 60)
    if (minutes < 60) return rtf.format(-minutes, "minute")
    const hours = Math.floor(minutes / 60)
    if (hours < 24) return rtf.format(-hours, "hour")
    const days = Math.floor(hours / 24)
    if (days < 30) return rtf.format(-days, "day")
    const months = Math.floor(days / 30)
    if (months < 12) return rtf.format(-months, "month")
    const years = Math.floor(months / 12)
    return rtf.format(-years, "year")
  } catch {
    return iso
  }
}

export function ChatHistory({
  open,
  onClose,
  state,
  recentAgents,
  onSelectAgent,
  onRemoveAgent,
  onNewConversation,
}: ChatHistoryProps) {
  const { t, i18n } = useTranslation()
  const reduceMotion = useReducedMotion()

  const isMock = recentAgents.length === 0
  const displayAgents = useMemo(
    () => (recentAgents.length > 0 ? recentAgents : getMockRecentAgents(t)),
    [recentAgents, t]
  )

  const currentAgent = useMemo(() => {
    if (!state.agentId) return null
    return recentAgents.find((agent) => agent.agentId === state.agentId) ?? null
  }, [recentAgents, state.agentId])

  const isRunning = state.view?.status === "running"

  useEffect(() => {
    if (!open) return
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose()
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [open, onClose])

  const panelVariants = {
    hidden: {
      scale: reduceMotion ? 1 : 0.96,
      x: reduceMotion ? 0 : 12,
      opacity: reduceMotion ? 1 : 0,
    },
    visible: { scale: 1, x: 0, opacity: 1 },
    exit: {
      scale: reduceMotion ? 1 : 0.96,
      x: reduceMotion ? 0 : 12,
      opacity: reduceMotion ? 1 : 0,
    },
  }

  const backdropVariants = {
    hidden: { opacity: 0 },
    visible: { opacity: 1 },
    exit: { opacity: 0 },
  }

  return (
    <AnimatePresence initial={false}>
      {open ? (
        <div key="chat-history-drawer" className="fixed inset-0 z-40">
          <div className="absolute inset-0 pointer-events-none" />

          <motion.aside
            id="chat-history-drawer"
            role="dialog"
            aria-modal="true"
            aria-label={t("chat.history.title")}
            initial="hidden"
            animate="visible"
            exit="exit"
            variants={panelVariants}
            transition={{
              duration: reduceMotion ? 0 : 0.22,
              ease: [0.16, 1, 0.3, 1] as const,
            }}
            className={cn(
              "wyse-history-drawer",
              "flex flex-col gap-2 overflow-hidden",
              "rounded-2xl border border-wyse-line bg-wyse-bg-paper shadow-wyse-soft",
              "max-h-[calc(100dvh-9rem)]"
            )}
          >
            <div className="flex items-center justify-between px-3 pt-2.5">
              <div className="flex items-center gap-1.5 text-muted-foreground">
                <HistoryIcon className="size-3" aria-hidden="true" />
                <span className="text-[10px] font-medium">
                  {t("chat.history.title")}
                </span>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                onClick={onClose}
                aria-label={t("errors.genericTitle")}
                className="text-muted-foreground hover:text-foreground"
              >
                <XIcon className="size-3" aria-hidden="true" />
              </Button>
            </div>

            {currentAgent ? (
              <button
                type="button"
                onClick={() => {
                  window.scrollTo({
                    top: document.body.scrollHeight,
                    behavior: reduceMotion ? "auto" : "smooth",
                  })
                }}
                className="mx-2.5 flex items-center gap-2 rounded-lg bg-secondary/50 px-2.5 py-2 text-left transition-colors hover:bg-secondary/70"
              >
                <div className="flex flex-col items-center gap-1">
                  <span
                    className={cn(
                      "size-1.5 shrink-0 rounded-full bg-wyse-action",
                      isRunning && "animate-pulse"
                    )}
                  />
                  <span className="w-px flex-1 bg-wyse-line/50" />
                </div>
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="text-[9px] font-medium text-wyse-action">
                    {t("chat.history.activeNow")}
                  </span>
                  <span className="truncate text-[11px] text-foreground">
                    {currentAgent.title}
                  </span>
                </div>
              </button>
            ) : null}

            <div className="px-2.5">
              <Button
                type="button"
                variant="ghost"
                aria-current={state.agentId === null ? "true" : undefined}
                className={cn(
                  "h-6 justify-start gap-1.5 rounded-lg px-2 text-[11px] font-medium",
                  state.agentId === null
                    ? "bg-wyse-action/10 text-wyse-action"
                    : "text-wyse-action hover:bg-wyse-action/5"
                )}
                onClick={() => {
                  onNewConversation()
                }}
              >
                <PlusIcon className="size-3" aria-hidden="true" />
                {t("chat.history.new")}
              </Button>
            </div>

            <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2.5 pb-2.5">
              <AnimatedList
                staggerDelay={0.025}
                maxDelay={0.18}
                className="gap-0.5"
              >
                {displayAgents.map((agent) => {
                  const isCurrent = agent.agentId === state.agentId
                  const isMissing = state.phase === "missing" && isCurrent
                  const isMockItem = isMock

                  return (
                    <div
                      key={agent.agentId}
                      className="group relative flex items-center"
                    >
                      <button
                        type="button"
                        disabled={isMockItem}
                        aria-current={isCurrent ? "true" : undefined}
                        onClick={() => {
                          onSelectAgent(agent.agentId)
                        }}
                        className={cn(
                          "flex min-w-0 flex-1 items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left transition-colors",
                          isCurrent
                            ? "bg-secondary/50"
                            : isMockItem
                              ? "opacity-50"
                              : "hover:bg-secondary/30"
                        )}
                      >
                        <span
                          className={cn(
                            "truncate text-[11px]",
                            isCurrent
                              ? "font-medium text-foreground"
                              : "text-foreground/80"
                          )}
                        >
                          {agent.title}
                        </span>
                        <span className="shrink-0 text-[9px] text-muted-foreground">
                          {formatRelativeTime(
                            agent.lastOpenedAt,
                            i18n.resolvedLanguage ?? "en"
                          )}
                        </span>
                      </button>

                      {isMissing && !isMockItem ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          aria-label={t("chat.removeLocalEntry")}
                          title={t("chat.removeLocalEntry")}
                          onClick={() => onRemoveAgent(agent.agentId)}
                          className="absolute -right-1 top-1/2 -translate-y-1/2 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                        >
                          <Trash2Icon
                            className="size-2.5 text-destructive"
                            aria-hidden="true"
                          />
                        </Button>
                      ) : null}
                    </div>
                  )
                })}
              </AnimatedList>
            </div>
          </motion.aside>
        </div>
      ) : null}
    </AnimatePresence>
  )
}
