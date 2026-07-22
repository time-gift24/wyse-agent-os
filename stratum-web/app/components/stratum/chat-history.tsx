"use client"

import { useEffect, useMemo, useRef } from "react"
import { HistoryIcon, PlusIcon, Trash2Icon, XIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { AnimatePresence, motion, useReducedMotion } from "motion/react"

import { Button } from "~/components/ui/button"
import type { ConversationState } from "~/features/agent-conversation/types"
import type { RecentAgent } from "~/lib/recent-agents"
import { cn } from "~/lib/utils"

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
  const panelRef = useRef<HTMLElement>(null)

  const currentAgent = useMemo(() => {
    if (!state.agentId) return null
    return recentAgents.find((agent) => agent.agentId === state.agentId) ?? null
  }, [recentAgents, state.agentId])

  const isRunning = state.view?.status === "running"

  useEffect(() => {
    if (!open) return
    const previouslyFocused = document.activeElement as HTMLElement | null
    const panel = panelRef.current
    const focusFrame = requestAnimationFrame(() => {
      panel?.querySelector<HTMLElement>("button:not(:disabled)")?.focus()
    })
    const previousOverflow = document.body.style.overflow
    document.body.style.overflow = "hidden"
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        onClose()
        return
      }
      if (event.key !== "Tab" || !panel) return
      const focusable = Array.from(
        panel.querySelectorAll<HTMLElement>(
          'button:not(:disabled), [href], [tabindex]:not([tabindex="-1"])'
        )
      )
      if (focusable.length === 0) return
      const first = focusable[0]
      const last = focusable.at(-1)
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last?.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first?.focus()
      }
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => {
      cancelAnimationFrame(focusFrame)
      window.removeEventListener("keydown", handleKeyDown)
      document.body.style.overflow = previousOverflow
      previouslyFocused?.focus()
    }
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

  return (
    <AnimatePresence initial={false}>
      {open ? (
        <div key="chat-history-drawer" className="fixed inset-0 z-[70]">
          <motion.div
            aria-hidden="true"
            className="absolute inset-0 cursor-default bg-stratum-ink/20"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: reduceMotion ? 0 : 0.18 }}
            onPointerDown={onClose}
          />

          <motion.aside
            ref={panelRef}
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
              "stratum-history-drawer",
              "flex flex-col gap-2 overflow-hidden",
              "rounded-2xl border border-stratum-line bg-popover shadow-stratum-soft",
              "max-h-[calc(100dvh-9rem)]"
            )}
          >
            <div className="flex items-center justify-between px-3 pt-2.5">
              <div className="flex items-center gap-2 text-muted-foreground">
                <HistoryIcon className="size-4" aria-hidden="true" />
                <span className="text-sm font-normal">
                  {t("chat.history.title")}
                </span>
              </div>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                onClick={onClose}
                aria-label={t("chat.history.close")}
                className="size-11 text-muted-foreground hover:text-foreground"
              >
                <XIcon className="size-4" aria-hidden="true" />
              </Button>
            </div>

            {currentAgent ? (
              <button
                type="button"
                onClick={() => {
                  onClose()
                  requestAnimationFrame(() => {
                    window.scrollTo({
                      top: document.body.scrollHeight,
                      behavior: reduceMotion ? "auto" : "smooth",
                    })
                  })
                }}
                className="mx-2.5 flex min-h-11 items-center gap-2 rounded-md bg-secondary/50 px-3 py-2 text-left transition-colors duration-200 hover:bg-secondary/70"
              >
                <div className="flex flex-col items-center gap-1">
                  <span
                    className={cn(
                      "size-1.5 shrink-0 rounded-full bg-stratum-action",
                      isRunning && "animate-pulse"
                    )}
                  />
                  <span className="w-px flex-1 bg-stratum-line/50" />
                </div>
                <div className="flex min-w-0 flex-1 flex-col">
                  <span className="text-sm font-semibold text-foreground">
                    {t("chat.history.activeNow")}
                  </span>
                  <span className="truncate text-sm text-foreground">
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
                  "min-h-11 w-full justify-start gap-2 rounded-md px-3 text-sm font-normal",
                  state.agentId === null
                    ? "bg-secondary text-foreground"
                    : "text-foreground hover:bg-secondary/70"
                )}
                onClick={() => {
                  onNewConversation()
                  onClose()
                }}
              >
                <PlusIcon className="size-4" aria-hidden="true" />
                {t("chat.history.new")}
              </Button>
            </div>

            <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-2.5 pb-2.5">
              {recentAgents.length === 0 ? (
                <p className="px-3 py-6 text-center text-sm text-muted-foreground">
                  {t("chat.history.empty")}
                </p>
              ) : (
                recentAgents.map((agent) => {
                  const isCurrent = agent.agentId === state.agentId
                  const isMissing = state.phase === "missing" && isCurrent

                  return (
                    <div
                      key={agent.agentId}
                      className="group relative flex items-center"
                    >
                      <button
                        type="button"
                        aria-current={isCurrent ? "true" : undefined}
                        onClick={() => {
                          onSelectAgent(agent.agentId)
                          onClose()
                        }}
                        className={cn(
                          "flex min-h-11 min-w-0 flex-1 items-center justify-between gap-2 rounded-md px-3 text-left text-sm transition-colors duration-200",
                          isMissing && "pr-12",
                          isCurrent
                            ? "bg-secondary/50"
                            : "hover:bg-secondary/30"
                        )}
                      >
                        <span
                          className={cn(
                            "truncate text-sm",
                            isCurrent
                              ? "font-semibold text-foreground"
                              : "text-foreground/80"
                          )}
                        >
                          {agent.title}
                        </span>
                        <span className="shrink-0 text-sm text-muted-foreground">
                          {formatRelativeTime(
                            agent.lastOpenedAt,
                            i18n.resolvedLanguage ?? "en"
                          )}
                        </span>
                      </button>

                      {isMissing ? (
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          aria-label={t("chat.removeLocalEntry")}
                          title={t("chat.removeLocalEntry")}
                          onClick={() => onRemoveAgent(agent.agentId)}
                          className="absolute top-1/2 right-0 size-11 shrink-0 -translate-y-1/2 text-destructive"
                        >
                          <Trash2Icon className="size-4" aria-hidden="true" />
                        </Button>
                      ) : null}
                    </div>
                  )
                })
              )}
            </div>
          </motion.aside>
        </div>
      ) : null}
    </AnimatePresence>
  )
}
