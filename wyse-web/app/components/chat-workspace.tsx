import { useRef, useState } from "react"
import {
  ArrowUpIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  Clock3Icon,
  PlusIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"

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
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "~/components/ui/message-scroller"
import { useAgentConversation } from "~/hooks/use-agent-conversation"

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
  const { state } = conversation
  const isAgentBusy =
    state.phase === "recovering" || state.view?.status === "running"

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
      className="h-[100dvh] overflow-hidden px-4 pt-4 pb-8 md:px-8 md:pb-10"
    >
      <div className="relative mx-auto flex h-full w-full max-w-5xl flex-col 2xl:block">
        <Card
          size="sm"
          className="relative mb-6 w-full shrink-0 bg-transparent ring-0 2xl:absolute 2xl:top-16 2xl:right-[calc(100%+1.5rem)] 2xl:mb-0 2xl:w-70"
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
              {isHistoryOpen ? (
                <ChevronDownIcon
                  aria-hidden="true"
                  className="size-4 shrink-0"
                />
              ) : (
                <ChevronRightIcon
                  aria-hidden="true"
                  className="size-4 shrink-0"
                />
              )}
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
          {isHistoryOpen ? (
            <CardContent id="chat-history" className="flex flex-col gap-1.5">
              {conversation.recentAgents.map((agent) => {
                const isMissing =
                  state.phase === "missing" && state.agentId === agent.agentId

                return (
                  <div key={agent.agentId} className="flex items-center gap-1">
                    <Button
                      variant={
                        state.agentId === agent.agentId ? "secondary" : "ghost"
                      }
                      size="lg"
                      className="h-auto min-w-0 flex-1 justify-start py-2 text-left"
                      onClick={() => conversation.selectAgent(agent.agentId)}
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
                        onClick={() =>
                          conversation.removeRecentAgent(agent.agentId)
                        }
                      >
                        {t("chat.removeLocalEntry")}
                      </Button>
                    ) : null}
                  </div>
                )
              })}
            </CardContent>
          ) : null}
        </Card>

        <div
          data-slot="chat-main"
          className="flex min-h-0 min-w-0 flex-1 flex-col pb-4 2xl:h-full"
        >
          <MessageScrollerProvider autoScroll>
            <MessageScroller className="flex-1">
              <MessageScrollerViewport>
                <MessageScrollerContent className="w-full px-1 py-6 md:px-6">
                  <AgentMessageList
                    messages={state.messages}
                    drafts={state.drafts}
                    tools={state.tools}
                  />
                  {Object.values(state.approvals).map((approval) => (
                    <MessageScrollerItem
                      key={approval.approvalId}
                      messageId={`approval:${approval.approvalId}`}
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
                </PromptInputFooter>
              </PromptInput>
            </CardContent>
          </Card>
        </div>
      </div>
    </section>
  )
}
