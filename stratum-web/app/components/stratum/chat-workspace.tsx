"use client"

import { useCallback, useEffect, useRef, useState } from "react"
import { ArrowDownIcon, ArrowUpIcon, BanIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { cn } from "~/lib/utils"

import { ChatHistory } from "~/components/stratum/chat-history"
import {
  AgentConfigMenu,
  ModelConfigMenu,
} from "~/components/stratum/model-config-menu"
import {
  type ApprovalDecision,
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
import { useAgentConversation } from "~/hooks/use-agent-conversation"

gsap.registerPlugin(useGSAP)

type ChatWorkspaceProps = {
  historyOpen?: boolean
  onHistoryOpenChange?(open: boolean): void
}

type AutoFollowScrollPosition = {
  paused: boolean
  previousScrollTop: number
  scrollTop: number
  targetScrollTop: number
}

const AUTO_FOLLOW_BOTTOM_EPSILON_PX = 1

function resolveAutoFollowPaused({
  paused,
  previousScrollTop,
  scrollTop,
  targetScrollTop,
}: AutoFollowScrollPosition) {
  const atBottom = targetScrollTop - scrollTop <= AUTO_FOLLOW_BOTTOM_EPSILON_PX
  if (paused) {
    return !(atBottom && scrollTop > previousScrollTop)
  }
  return scrollTop < previousScrollTop && !atBottom
}

export function ChatWorkspace({
  historyOpen = false,
  onHistoryOpenChange,
}: ChatWorkspaceProps) {
  const { t } = useTranslation()
  const conversation = useAgentConversation()
  const [composerText, setComposerText] = useState("")
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [autoFollowPaused, setAutoFollowPaused] = useState(false)
  const [approvalSubmissions, setApprovalSubmissions] = useState<
    ReadonlyMap<string, ApprovalDecision>
  >(() => new Map())
  const composerRef = useRef<HTMLTextAreaElement>(null)
  const submitButtonRef = useRef<HTMLDivElement>(null)
  const workspaceRef = useRef<HTMLElement>(null)
  const messageListRef = useRef<HTMLDivElement>(null)
  const inputContainerRef = useRef<HTMLDivElement>(null)
  const composerSurfaceRef = useRef<HTMLDivElement>(null)
  const autoFollowPausedRef = useRef(false)
  const previousScrollTopRef = useRef(0)

  const { state, recentAgents, selectAgent, removeRecentAgent } = conversation
  const isNewConversation = state.agentId === null
  const initialComposerBottom = useRef(
    isNewConversation ? "50%" : "max(1rem, env(safe-area-inset-bottom))"
  )
  const isAgentBusy =
    state.phase === "recovering" || state.view?.status === "running"
  const composerRunning = isSubmitting || isAgentBusy
  const canCancel = state.agentId !== null && state.view?.status === "running"
  const liveStatus = isSubmitting
    ? t("chat.sending")
    : state.phase === "recovering"
      ? t("chat.connecting")
      : state.phase === "connection_error"
        ? t("chat.connectionFailed")
        : state.view?.status === "running"
          ? t("chat.thinking")
          : t("chat.ready")

  // 选择对话（包括新建）后聚焦输入框
  useEffect(() => {
    const timer = setTimeout(() => {
      composerRef.current?.focus()
    }, 100)
    return () => clearTimeout(timer)
  }, [state.agentId])

  const scrollToBottom = useCallback((behavior: ScrollBehavior) => {
    if (typeof document === "undefined") return
    const scrollElement = document.documentElement
    scrollElement.scrollTo({
      top: Math.max(scrollElement.scrollHeight - scrollElement.clientHeight, 0),
      behavior,
    })
  }, [])

  const resumeAutoFollow = useCallback(
    (behavior: ScrollBehavior) => {
      autoFollowPausedRef.current = false
      setAutoFollowPaused(false)
      scrollToBottom(behavior)
    },
    [scrollToBottom]
  )
  const closeHistory = useCallback(
    () => onHistoryOpenChange?.(false),
    [onHistoryOpenChange]
  )

  useEffect(() => {
    if (typeof document === "undefined") return
    const scrollElement = document.documentElement
    previousScrollTopRef.current = scrollElement.scrollTop

    const handleScroll = () => {
      const scrollTop = scrollElement.scrollTop
      const previousScrollTop = previousScrollTopRef.current
      const paused = resolveAutoFollowPaused({
        paused: autoFollowPausedRef.current,
        previousScrollTop,
        scrollTop,
        targetScrollTop: Math.max(
          scrollElement.scrollHeight - scrollElement.clientHeight,
          0
        ),
      })
      previousScrollTopRef.current = scrollTop
      if (paused !== autoFollowPausedRef.current) {
        autoFollowPausedRef.current = paused
        setAutoFollowPaused(paused)
      }
    }

    document.addEventListener("scroll", handleScroll, { passive: true })
    return () => document.removeEventListener("scroll", handleScroll)
  }, [])

  useEffect(() => {
    const messageList = messageListRef.current
    if (!messageList || typeof ResizeObserver === "undefined") return
    let scrollFrame: number | undefined
    const resizeObserver = new ResizeObserver(() => {
      if (autoFollowPausedRef.current) return
      if (scrollFrame !== undefined) cancelAnimationFrame(scrollFrame)
      scrollFrame = requestAnimationFrame(() => scrollToBottom("auto"))
    })
    resizeObserver.observe(messageList)
    return () => {
      if (scrollFrame !== undefined) cancelAnimationFrame(scrollFrame)
      resizeObserver.disconnect()
    }
  }, [scrollToBottom])

  useEffect(() => {
    resumeAutoFollow("auto")
  }, [resumeAutoFollow, state.agentId])

  useGSAP(
    () => {
      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const messageList = messageListRef.current
      const composerSurface = composerSurfaceRef.current
      if (!messageList || !composerSurface) return

      const tl = gsap.timeline()
      tl.fromTo(
        messageList,
        { autoAlpha: 0, y: 8 },
        {
          autoAlpha: 1,
          y: 0,
          duration: reduceMotion ? 0 : 0.2,
          ease: "power2.out",
        }
      ).fromTo(
        composerSurface,
        { autoAlpha: 0, y: 8 },
        {
          autoAlpha: 1,
          y: 0,
          duration: reduceMotion ? 0 : 0.2,
          ease: "power2.out",
        },
        reduceMotion ? 0 : "-=0.12"
      )
    },
    { scope: workspaceRef }
  )

  useGSAP(
    () => {
      const container = inputContainerRef.current
      if (!container) return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const previousTop = container.getBoundingClientRect().top
      const bottom = isNewConversation
        ? "50%"
        : "max(1rem, env(safe-area-inset-bottom))"

      gsap.set(container, { bottom, y: 0 })
      const nextTop = container.getBoundingClientRect().top
      if (reduceMotion) return

      gsap.fromTo(
        container,
        { y: previousTop - nextTop, willChange: "transform" },
        {
          y: 0,
          duration: 0.22,
          ease: "power2.inOut",
          clearProps: "transform,willChange",
        }
      )
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
        gsap.to(btn, { scale: 0.94, duration: 0.15, ease: "power2.out" })
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
    setApprovalSubmissions((submissions) =>
      startApprovalSubmission(submissions, approvalId, decision)
    )
    try {
      await conversation.resolveApproval(approvalId, decision)
    } finally {
      setApprovalSubmissions((submissions) =>
        finishApprovalSubmission(submissions, approvalId)
      )
    }
  }

  return (
    <section
      ref={workspaceRef}
      id="longzhong"
      className="min-h-[100dvh] w-full px-4 pt-20 pb-[calc(13rem+env(safe-area-inset-bottom))] md:px-8 md:pt-24 md:pb-[calc(14rem+env(safe-area-inset-bottom))]"
    >
      <ChatHistory
        open={historyOpen}
        onClose={closeHistory}
        state={state}
        recentAgents={recentAgents}
        onSelectAgent={selectAgent}
        onRemoveAgent={removeRecentAgent}
        onNewConversation={() => selectAgent(null)}
      />

      <div className="stratum-content-width mx-auto">
        <div data-slot="chat-main" className="flex min-w-0 flex-col">
          <div
            ref={messageListRef}
            data-slot="chat-message-list"
            role="log"
            aria-live={state.phase === "recovering" ? "off" : "polite"}
            aria-relevant="additions text"
            className="type-body w-full px-1 py-6 [overflow-anchor:none] md:px-6"
          >
            <AgentMessageList
              messages={state.messages}
              drafts={state.drafts}
              tools={state.tools}
              approvals={state.approvals}
              approvalSubmissions={approvalSubmissions}
              onApprovalDecision={(approvalId, decision) => {
                void resolveApproval(approvalId, decision)
              }}
              error={state.error}
            />
          </div>
        </div>
      </div>

      {autoFollowPaused && (
        <button
          type="button"
          onClick={() => resumeAutoFollow("smooth")}
          className="fixed bottom-[calc(var(--stratum-composer-min-height)+max(1.75rem,env(safe-area-inset-bottom)))] left-1/2 z-50 size-11 -translate-x-1/2 rounded-full border border-stratum-line-strong bg-stratum-paper text-foreground shadow-stratum-soft transition-transform duration-200 hover:-translate-y-0.5 motion-reduce:transition-none"
          aria-label={t("chat.scrollToBottom")}
        >
          <ArrowDownIcon className="size-4" aria-hidden="true" />
        </button>
      )}

      <div
        ref={inputContainerRef}
        data-slot="chat-composer-positioner"
        data-composer-position={isNewConversation ? "centered" : "docked"}
        className="stratum-composer-width fixed inset-x-0 z-40 mx-auto"
        style={{ bottom: initialComposerBottom.current }}
      >
        <div
          ref={composerSurfaceRef}
          data-slot="chat-composer-surface"
          className={cn(
            "transition-transform duration-200 ease-in-out motion-reduce:transition-none",
            isNewConversation ? "translate-y-1/2" : "translate-y-0"
          )}
        >
          <div className="stratum-prompt-shell relative">
            <PromptInput
              aria-busy={composerRunning}
              className="[&_[data-slot=input-group]]:min-h-[var(--stratum-composer-min-height)] [&_[data-slot=input-group]]:rounded-[var(--radius-stratum-panel)]! [&_[data-slot=input-group]]:border-stratum-line-strong! [&_[data-slot=input-group]]:bg-stratum-paper! [&_[data-slot=input-group]]:shadow-none! [&_[data-slot=input-group]]:backdrop-blur-none!"
              onSubmit={(event) => {
                event.preventDefault()
                void submitMessage()
              }}
            >
              <PromptInputBody>
                <PromptInputTextarea
                  ref={composerRef}
                  aria-label={t("chat.composer.label")}
                  className="max-h-48 min-h-14 px-4 pt-3 pb-2 text-base! leading-6! placeholder:text-muted-foreground md:px-5"
                  disabled={composerRunning}
                  onChange={(event) => setComposerText(event.target.value)}
                  placeholder={t("chat.composer.placeholder")}
                  value={composerText}
                />
              </PromptInputBody>
              <PromptInputFooter className="min-h-11 gap-2 px-2 pt-0 pb-[max(0.5rem,env(safe-area-inset-bottom))] sm:px-3">
                <PromptInputTools className="[scrollbar-width:none] gap-0.5 overflow-x-auto [&::-webkit-scrollbar]:hidden">
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
                      className="shrink-0"
                      variant="outline"
                      onClick={() => conversation.reconnect()}
                    >
                      {t("chat.reconnect")}
                    </PromptInputButton>
                  ) : null}
                </PromptInputTools>
                <div
                  ref={submitButtonRef}
                  className="inline-flex shrink-0 items-center gap-1"
                >
                  <PromptInputSubmit
                    aria-label={t(
                      canCancel ? "chat.cancel" : "chat.composer.send"
                    )}
                    className={cn(
                      "size-11 shrink-0 active:translate-y-px",
                      !canCancel &&
                        composerText.trim() === "" &&
                        "bg-muted text-muted-foreground hover:bg-muted"
                    )}
                    disabled={
                      !canCancel &&
                      (composerRunning || composerText.trim() === "")
                    }
                    onClick={
                      canCancel ? () => void conversation.cancel() : undefined
                    }
                    type={canCancel ? "button" : "submit"}
                  >
                    {canCancel ? (
                      <BanIcon aria-hidden="true" />
                    ) : (
                      <ArrowUpIcon aria-hidden="true" />
                    )}
                  </PromptInputSubmit>
                </div>
              </PromptInputFooter>
            </PromptInput>
            {composerRunning ? (
              <span
                aria-hidden="true"
                className="pointer-events-none absolute top-0 left-1/2 h-0.5 w-16 -translate-x-1/2 animate-pulse rounded-full bg-stratum-action motion-reduce:animate-none"
              />
            ) : null}
          </div>
          <p className="sr-only" role="status" aria-live="polite">
            {liveStatus}
          </p>
        </div>
      </div>
    </section>
  )
}
