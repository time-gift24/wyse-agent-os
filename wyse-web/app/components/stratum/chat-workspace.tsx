"use client"

import { useRef, useState } from "react"
import { ArrowUpIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"

import { AgentApprovalCard } from "~/components/stratum/agent-approval-card"
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
} from "~/components/stratum/agent-approval-submissions"
import { AgentMessageList } from "~/components/stratum/agent-message-list"
import { Card, CardContent } from "~/components/ui/card"
import { useAgentConversation } from "~/hooks/use-agent-conversation"

gsap.registerPlugin(useGSAP)

export function ChatWorkspace() {
  const { t } = useTranslation()
  const conversation = useAgentConversation()
  const [composerText, setComposerText] = useState("")
  const [isSubmitting, setIsSubmitting] = useState(false)
  const [submittingApprovalIds, setSubmittingApprovalIds] = useState<
    ReadonlySet<string>
  >(() => new Set())
  const composerRef = useRef<HTMLTextAreaElement>(null)
  const submitButtonRef = useRef<HTMLDivElement>(null)
  const { state } = conversation
  const isAgentBusy =
    state.phase === "recovering" || state.view?.status === "running"
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
      id="longzhong"
      className="min-h-[100dvh] w-full px-4 pt-20 pb-52 md:px-8 md:pt-24 md:pb-56"
    >
      <div className="wyse-content-width mx-auto">
        <div data-slot="chat-main" className="flex min-w-0 flex-col">
          <div
            data-slot="chat-message-list"
            className="w-full px-1 py-6 md:px-6"
          >
            <AgentMessageList
              messages={state.messages}
              drafts={state.drafts}
              tools={state.tools}
              failure={state.failure}
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

      <div className="fixed inset-x-0 bottom-4 z-40 px-4 md:bottom-6 md:px-8">
        <Card
          size="sm"
          className="wyse-content-width mx-auto bg-transparent ring-0"
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
                      isSubmitting || isAgentBusy || composerText.trim() === ""
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
    </section>
  )
}
