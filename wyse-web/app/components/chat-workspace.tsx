"use client"

import { useRef, useState } from "react"
import {
  ArrowUpIcon,
  ChevronDownIcon,
  Clock3Icon,
  PlusIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"

import { AgentApprovalCard } from "~/components/agent-approval-card"
import {
  PromptInput,
  PromptInputBody,
  PromptInputButton,
  PromptInputFooter,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputTools,
} from "~/components/ai-elements/prompt-input"
import {
  finishApprovalSubmission,
  startApprovalSubmission,
} from "~/components/agent-approval-submissions"
import { AgentMessageList } from "~/components/agent-message-list"
import GlassSurface from "~/components/GlassSurface"
import { Button } from "~/components/ui/button"
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "~/components/ui/card"
import { Separator } from "~/components/ui/separator"
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "~/components/ui/message-scroller"
import { useAgentConversation } from "~/hooks/use-agent-conversation"

gsap.registerPlugin(useGSAP)

export function ChatWorkspace() {
  const { t } = useTranslation()
  const conversation = useAgentConversation()
  const [isHistoryOpen, setIsHistoryOpen] = useState(false)
  const [composerText, setComposerText] = useState("")
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [submittingApprovalIds, setSubmittingApprovalIds] = useState<
    ReadonlySet<string>
  >(() => new Set())
  const composerRef = useRef<HTMLTextAreaElement>(null)
  const historyContentRef = useRef<HTMLDivElement>(null)
  const submitButtonRef = useRef<HTMLDivElement>(null)
  const historyInitialized = useRef(false)
  const { state } = conversation
  const isAgentBusy =
    state.phase === "recovering" || state.view?.status === "running"
  const activeAgent =
    state.agentId === null
      ? undefined
      : conversation.recentAgents.find(
          (agent) => agent.agentId === state.agentId
        )
  const historicalAgents = activeAgent
    ? conversation.recentAgents.filter(
        (agent) => agent.agentId !== activeAgent.agentId
      )
    : conversation.recentAgents

  useGSAP(
    () => {
      const wrapper = historyContentRef.current
      if (!wrapper) return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches

      if (!historyInitialized.current) {
        historyInitialized.current = true
        gsap.set(wrapper, {
          height: isHistoryOpen ? "auto" : 0,
          opacity: isHistoryOpen ? 1 : 0,
        })
        return
      }

      if (reduceMotion) {
        gsap.set(wrapper, {
          height: isHistoryOpen ? "auto" : 0,
          opacity: isHistoryOpen ? 1 : 0,
        })
        return
      }

      if (isHistoryOpen) {
        gsap.to(wrapper, {
          height: "auto",
          opacity: 1,
          duration: 0.25,
          ease: "power2.out",
        })
        const items = wrapper.querySelectorAll<HTMLElement>(
          "[data-history-item]"
        )
        if (items.length > 0) {
          gsap.fromTo(
            items,
            { opacity: 0, y: 8 },
            {
              opacity: 1,
              y: 0,
              duration: 0.2,
              stagger: 0.04,
              ease: "power2.out",
              delay: 0.05,
            }
          )
        }
      } else {
        gsap.to(wrapper, {
          height: 0,
          opacity: 0,
          duration: 0.2,
          ease: "power2.in",
        })
      }
    },
    { dependencies: [isHistoryOpen] }
  )

  useGSAP(
    () => {
      const btn = submitButtonRef.current
      if (!btn) return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      if (reduceMotion) {
        gsap.set(btn, { scale: 1 })
        return
      }

      if (isSubmitting) {
        gsap.to(btn, { scale: 0.92, duration: 0.1, ease: "power2.out" })
      } else {
        gsap.to(btn, { scale: 1, duration: 0.2, ease: "expo.out" })
      }
    },
    { dependencies: [isSubmitting] }
  )

  const renderConversationEntry = (
    agent: (typeof conversation.recentAgents)[number],
    onSelect?: () => void
  ) => {
    const isCurrent = agent.agentId === state.agentId
    const isMissing = state.phase === "missing" && isCurrent

    return (
      <div key={agent.agentId} className="flex items-center gap-1">
        <Button
          variant={isCurrent ? "secondary" : "ghost"}
          size="lg"
          className="h-auto min-w-0 flex-1 justify-start py-2 text-left"
          onClick={() => {
            conversation.selectAgent(agent.agentId)
            onSelect?.()
          }}
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
            onClick={() => conversation.removeRecentAgent(agent.agentId)}
          >
            {t("chat.removeLocalEntry")}
          </Button>
        ) : null}
      </div>
    )
  }

  const submitMessage = async () => {
    const text = composerText.trim()
    if (text === "" || isSubmitting || isAgentBusy) return

    setIsSubmitting(true)
    try {
      const sent =
        state.agentId === null
          ? await conversation.createConversation(text)
          : await conversation.sendMessage(text)
      if (sent) setComposerText("")
    } finally {
      setIsSubmitting(false)
    }
  }

  const resolveApproval = async (
    approvalId: string,
    decision: "approve" | "reject"
  ) => {
    setSubmittingApprovalIds((approvalIds) =>
      startApprovalSubmission(approvalIds, approvalId)
    )
    try {
      await conversation.resolveApproval(approvalId, decision)
    } finally {
      setSubmittingApprovalIds((approvalIds) =>
        finishApprovalSubmission(approvalIds, approvalId)
      )
    }
  }

  return (
    <section
      id="longzhong"
      className="h-[100dvh] w-screen shrink-0 overflow-visible px-4 pt-4 pb-8 md:px-8 md:pb-10"
    >
      <div className="wyse-content-width relative mx-auto flex h-full flex-col gap-(--layout-gap)">
        <Card
          size="sm"
          className="wyse-history-rail relative mb-6 shrink-0 bg-transparent ring-0 lg:mb-0"
        >
          <div className="absolute inset-0 -z-10">
            <GlassSurface
              width="100%"
              height="100%"
              borderRadius={8}
              borderWidth={0.06}
              brightness={68}
              opacity={0.94}
              blur={10}
              displace={0}
              backgroundOpacity={0.45}
              saturation={1.15}
              distortionScale={-40}
              redOffset={0}
              greenOffset={2}
              blueOffset={4}
              mixBlendMode="normal"
            />
          </div>

          <CardHeader className="grid-cols-[minmax(0,1fr)_auto] grid-rows-[auto] items-center gap-2">
            <button
              type="button"
              aria-controls="chat-history"
              aria-expanded={isHistoryOpen}
              className="-m-1 flex min-w-0 items-center gap-1 rounded-sm p-1 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
              onClick={() => setIsHistoryOpen((open) => !open)}
            >
              <CardTitle className="truncate">
                {t("chat.history.title")}
              </CardTitle>
              <ChevronDownIcon
                aria-hidden="true"
                className="size-4 shrink-0 transition-transform duration-200 ease-out"
                style={{
                  transform: isHistoryOpen ? "rotate(0deg)" : "rotate(-90deg)",
                }}
              />
            </button>
            <CardAction className="col-start-2 row-span-1 row-start-1 self-center">
              <Button
                variant="outline"
                size="icon-sm"
                aria-label={t("chat.history.new")}
                title={t("chat.history.new")}
                onClick={() => {
                  conversation.selectAgent(null)
                  composerRef.current?.focus()
                }}
              >
                <PlusIcon aria-hidden="true" />
              </Button>
            </CardAction>
          </CardHeader>
          {activeAgent ? (
            <CardContent data-slot="active-conversation" className="pt-0">
              {renderConversationEntry(activeAgent)}
            </CardContent>
          ) : null}
          {activeAgent && historicalAgents.length > 0 ? (
            <div
              ref={historyContentRef}
              className="overflow-hidden"
              aria-hidden={!isHistoryOpen}
            >
              <Separator
                data-slot="history-divider"
                className="mx-(--card-spacing) w-auto"
              />
              <CardContent
                id="chat-history"
                data-slot="history-conversations"
                className="flex flex-col gap-1.5"
              >
                {historicalAgents.map((agent) => (
                  <div key={agent.agentId} data-history-item>
                    {renderConversationEntry(agent, () =>
                      setIsHistoryOpen(false)
                    )}
                  </div>
                ))}
              </CardContent>
            </div>
          ) : null}
        </Card>

        <div
          data-slot="chat-main"
          className="flex min-h-0 min-w-0 flex-1 flex-col pb-4"
        >
          <MessageScrollerProvider autoScroll>
            <MessageScroller className="flex-1">
              <MessageScrollerViewport>
                <MessageScrollerContent className="w-full px-1 py-6 md:px-6">
                  <AgentMessageList
                    messages={state.messages}
                    drafts={state.drafts}
                    tools={state.tools}
                    failure={state.failure}
                  />
                  {Object.values(state.approvals).map((approval) => (
                    <MessageScrollerItem
                      key={approval.approvalId}
                      messageId={`approval:${approval.approvalId}`}
                      className="animate-in duration-300 fade-in-0 slide-in-from-bottom-3 zoom-in-[0.96]"
                    >
                      <AgentApprovalCard
                        approval={approval}
                        submitting={submittingApprovalIds.has(
                          approval.approvalId
                        )}
                        onDecision={(decision) => {
                          void resolveApproval(approval.approvalId, decision)
                        }}
                      />
                    </MessageScrollerItem>
                  ))}
                </MessageScrollerContent>
              </MessageScrollerViewport>
              <MessageScrollerButton />
            </MessageScroller>
          </MessageScrollerProvider>

          <Card size="sm" className="w-full shrink-0 bg-transparent ring-0">
            <CardContent>
              <PromptInput
                onSubmit={(event) => {
                  event.preventDefault()
                  void submitMessage()
                }}
              >
                <PromptInputBody>
                  <PromptInputTextarea
                    ref={composerRef}
                    aria-label={t("chat.composer.label")}
                    disabled={isSubmitting || isAgentBusy}
                    onChange={(event) => setComposerText(event.target.value)}
                    placeholder={t("chat.composer.placeholder")}
                    value={composerText}
                  />
                </PromptInputBody>
                <PromptInputFooter>
                  <PromptInputTools>
                    {state.phase === "connection_error" ? (
                      <PromptInputButton
                        variant="outline"
                        onClick={() => conversation.reconnect()}
                      >
                        {t("chat.reconnect")}
                      </PromptInputButton>
                    ) : state.agentId !== null && isAgentBusy ? (
                      <PromptInputButton
                        variant="outline"
                        onClick={() => void conversation.cancel()}
                      >
                        {t("chat.cancel")}
                      </PromptInputButton>
                    ) : null}
                  </PromptInputTools>
                  <div ref={submitButtonRef} className="inline-flex">
                    <PromptInputSubmit
                      aria-label={t("chat.composer.send")}
                      className={
                        composerText.trim() === ""
                          ? "bg-muted text-muted-foreground hover:bg-muted"
                          : undefined
                      }
                      disabled={
                        isSubmitting ||
                        isAgentBusy ||
                        composerText.trim() === ""
                      }
                    >
                      <ArrowUpIcon aria-hidden="true" />
                    </PromptInputSubmit>
                  </div>
                </PromptInputFooter>
              </PromptInput>
            </CardContent>
          </Card>
        </div>
      </div>
    </section>
  )
}
