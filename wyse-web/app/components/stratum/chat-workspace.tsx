"use client"

import { useEffect, useRef, useState } from "react"
import { ArrowDownIcon, ArrowUpIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { useStickToBottom } from "use-stick-to-bottom"
import { cn } from "~/lib/utils"

import { AgentApprovalCard } from "~/components/stratum/agent-approval-card"
import { ChatHistory } from "~/components/stratum/chat-history"
import {
  AgentConfigMenu,
  ModelConfigMenu,
} from "~/components/stratum/model-config-menu"
import {
  finishApprovalSubmission,
  startApprovalSubmission,
} from "~/components/stratum/agent-approval-submissions"
import { AgentMessageList } from "~/components/stratum/agent-message-list"
import {
  PromptInput,
  PromptInputBody,
  PromptInputButton,
  PromptInputFooter,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputTools,
} from "~/components/ai-elements/prompt-input"
import { Card, CardContent } from "~/components/ui/card"
import { useAgentConversation } from "~/hooks/use-agent-conversation"

gsap.registerPlugin(useGSAP)

type ChatWorkspaceProps = {
  historyOpen?: boolean
  onHistoryOpenChange?(open: boolean): void
}

export function ChatWorkspace({
  historyOpen = false,
  onHistoryOpenChange,
}: ChatWorkspaceProps) {
  const { t } = useTranslation()
  const conversation = useAgentConversation()
  const [composerText, setComposerText] = useState("")
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [submittingApprovalIds, setSubmittingApprovalIds] = useState<
    ReadonlySet<string>
  >(() => new Set())
  const composerRef = useRef<HTMLTextAreaElement>(null)
  const submitButtonRef = useRef<HTMLDivElement>(null)
  const workspaceRef = useRef<HTMLElement>(null)
  const messageListRef = useRef<HTMLDivElement>(null)
  const inputContainerRef = useRef<HTMLDivElement>(null)

  const { state, recentAgents, selectAgent, removeRecentAgent } = conversation
  const isNewConversation = state.agentId === null
  const initialComposerBottom = useRef(isNewConversation ? "50%" : "0px")
  const isAgentBusy =
    state.phase === "recovering" || state.view?.status === "running"

  // 选择对话（包括新建）后聚焦输入框
  useEffect(() => {
    const timer = setTimeout(() => {
      composerRef.current?.focus()
    }, 100)
    return () => clearTimeout(timer)
  }, [state.agentId])

  const { scrollRef, contentRef, scrollToBottom, isAtBottom } =
    useStickToBottom({
      initial: "smooth",
      resize: "smooth",
    })

  useEffect(() => {
    if (typeof document === "undefined") return
    scrollRef(document.documentElement)
    return () => {
      scrollRef(null as unknown as HTMLElement)
    }
  }, [scrollRef])

  // workspace 入场动画：与 navbar 收缩完全同步
  useGSAP(
    () => {
      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const messageList = messageListRef.current
      const inputContainer = inputContainerRef.current
      if (!messageList || !inputContainer) return

      gsap.set([messageList, inputContainer], {
        autoAlpha: 0,
        y: 12,
      })

      const tl = gsap.timeline({ delay: 0.12 }) // 与 navbar timeline 错开 120ms
      tl.to(messageList, {
        autoAlpha: 1,
        y: 0,
        duration: reduceMotion ? 0 : 0.45,
        ease: "sine.out",
      })
      tl.to(
        inputContainer,
        {
          autoAlpha: 1,
          y: 0,
          duration: reduceMotion ? 0 : 0.35,
          ease: "sine.out",
        },
        "-=0.3"
      )
    },
    { scope: workspaceRef }
  )

  // 输入框位置切换动画：居中 <-> 底部
  // 只用 bottom 属性控制，避免 top/bottom 切换导致的跳动
  useGSAP(
    () => {
      const container = inputContainerRef.current
      if (!container) return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches

      if (isNewConversation) {
        // 居中状态 - 内层负责垂直偏移，避免与入场动画争用 transform。
        gsap.to(container, {
          bottom: "50%",
          duration: reduceMotion ? 0 : 0.5,
          ease: "sine.inOut",
        })
      } else {
        // 底部状态
        gsap.to(container, {
          bottom: 0,
          duration: reduceMotion ? 0 : 0.5,
          ease: "sine.inOut",
        })
      }
    },
    { dependencies: [isNewConversation], scope: workspaceRef }
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
      ref={workspaceRef}
      id="longzhong"
      className="min-h-[100dvh] w-full px-4 pt-20 pb-52 md:px-8 md:pt-24 md:pb-56"
    >
      <ChatHistory
        open={historyOpen}
        onClose={() => onHistoryOpenChange?.(false)}
        state={state}
        recentAgents={recentAgents}
        onSelectAgent={selectAgent}
        onRemoveAgent={removeRecentAgent}
        onNewConversation={() => selectAgent(null)}
      />

      <div className="wyse-content-width mx-auto">
        <div data-slot="chat-main" className="flex min-w-0 flex-col">
          <div
            ref={(node) => {
              messageListRef.current = node
              contentRef(node)
            }}
            data-slot="chat-message-list"
            className="w-full px-1 py-6 md:px-6"
          >
            <AgentMessageList
              messages={state.messages}
              drafts={state.drafts}
              tools={state.tools}
              error={state.error}
            />
            {Object.values(state.approvals).map((approval) => (
              <div
                key={approval.approvalId}
                className="animate-in duration-300 fade-in-0 slide-in-from-bottom-3 zoom-in-[0.96]"
              >
                <AgentApprovalCard
                  approval={approval}
                  submitting={submittingApprovalIds.has(approval.approvalId)}
                  onDecision={(decision) => {
                    void resolveApproval(approval.approvalId, decision)
                  }}
                />
              </div>
            ))}
          </div>
        </div>
      </div>

      {!isAtBottom && (
        <button
          type="button"
          onClick={() => scrollToBottom()}
          className="fixed bottom-28 left-1/2 z-50 -translate-x-1/2 rounded-full border border-border bg-background/90 p-2 text-foreground shadow-wyse-soft transition-transform hover:scale-105"
          aria-label={t("chat.scrollToBottom")}
        >
          <ArrowDownIcon className="size-4" aria-hidden="true" />
        </button>
      )}

      <div
        ref={inputContainerRef}
        className="wyse-content-width fixed inset-x-0 z-40 mx-auto px-4 md:px-0"
        style={{ bottom: initialComposerBottom.current }}
      >
        <div
          className={cn(
            "transition-transform duration-500 ease-in-out motion-reduce:transition-none",
            isNewConversation ? "-translate-y-1/2" : "translate-y-0"
          )}
        >
          <Card
            size="sm"
            className="prompt-input-glass wyse-content-width mx-auto bg-transparent ring-0"
          >
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
                    <AgentConfigMenu
                      configuration={conversation.composerConfiguration}
                      commandPending={isSubmitting}
                    />
                    <ModelConfigMenu
                      configuration={conversation.composerConfiguration}
                      commandPending={isSubmitting}
                    />
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
                  <div
                    ref={submitButtonRef}
                    className="inline-flex items-center gap-1"
                  >
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
