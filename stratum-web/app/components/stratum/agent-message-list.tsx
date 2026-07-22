import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { BrainIcon } from "lucide-react"

import {
  Message,
  MessageContent,
  MessageResponse,
} from "~/components/ai-elements/message"
import { ReasoningContent } from "~/components/ai-elements/reasoning"
import { Shimmer } from "~/components/ai-elements/shimmer"
import { AgentApprovalCard } from "~/components/stratum/agent-approval-card"
import type { ApprovalDecision } from "~/components/stratum/agent-approval-submissions"
import { AgentDisclosure } from "~/components/stratum/agent-disclosure"
import { ToolTraceRow } from "~/components/stratum/agent-tool-trace"
import type {
  ApprovalRequest,
  StableMessage,
  ToolProgress,
} from "~/features/agent-conversation/types"
import type { ApiError } from "~/lib/stratum-api"

type AgentMessageListProps = {
  messages: readonly StableMessage[]
  drafts: Readonly<Record<string, { text: string; reasoning: string }>>
  tools: Readonly<Record<string, ToolProgress>>
  approvals: Readonly<Record<string, ApprovalRequest>>
  approvalSubmissions: ReadonlyMap<string, ApprovalDecision>
  onApprovalDecision(approvalId: string, decision: ApprovalDecision): void
  error?: ApiError | null
}

type ReasoningDisclosureProps = {
  children: string
  isStreaming?: boolean
  getThinkingMessage(isStreaming: boolean): ReactNode
}

function ReasoningDisclosure({
  children,
  isStreaming = false,
  getThinkingMessage,
}: ReasoningDisclosureProps) {
  return (
    <AgentDisclosure
      icon={<BrainIcon aria-hidden="true" className="size-4" />}
      label={getThinkingMessage(isStreaming)}
    >
      <ReasoningContent>{children}</ReasoningContent>
    </AgentDisclosure>
  )
}

type ExecutionTraceGroupProps = {
  tools: readonly ToolProgress[]
  approvalsByCallId: ReadonlyMap<string, ApprovalRequest>
  approvalSubmissions: ReadonlyMap<string, ApprovalDecision>
  onApprovalDecision(approvalId: string, decision: ApprovalDecision): void
}

function ExecutionTraceGroup({
  tools,
  approvalsByCallId,
  approvalSubmissions,
  onApprovalDecision,
}: ExecutionTraceGroupProps) {
  if (tools.length === 0) return null

  return (
    <div className="mt-2 flex flex-col">
      {tools.map((tool) => {
        const approval = approvalsByCallId.get(tool.callId)
        return (
          <ToolTraceRow key={tool.callId} tool={tool}>
            {approval ? (
              <AgentApprovalCard
                approval={approval}
                submittingDecision={
                  approvalSubmissions.get(approval.approvalId) ?? null
                }
                onDecision={(decision) =>
                  onApprovalDecision(approval.approvalId, decision)
                }
              />
            ) : null}
          </ToolTraceRow>
        )
      })}
    </div>
  )
}

export function AgentMessageList({
  messages,
  drafts,
  tools,
  approvals,
  approvalSubmissions,
  onApprovalDecision,
  error = null,
}: AgentMessageListProps) {
  const { t, i18n } = useTranslation()
  const dateTimeFormat = new Intl.DateTimeFormat(i18n.resolvedLanguage, {
    dateStyle: "short",
    timeStyle: "short",
  })
  const thinkingMessage = (isStreaming: boolean) =>
    isStreaming ? (
      <Shimmer as="span" duration={1.4}>
        {t("chat.thinking")}
      </Shimmer>
    ) : (
      t("chat.reasoningComplete")
    )
  const toolValues = Object.values(tools)
  const approvalValues = Object.values(approvals)
  const approvalsByCallId = new Map(
    approvalValues.map((approval) => [approval.callId, approval])
  )
  const stableToolCallIds = new Set(
    messages.flatMap((message) =>
      message.toolCalls.map((toolCall) => toolCall.callId)
    )
  )
  const draftIds = new Set(Object.keys(drafts))
  const knownToolCallIds = new Set(toolValues.map((tool) => tool.callId))
  const orphanTools = toolValues.filter(
    (tool) =>
      !stableToolCallIds.has(tool.callId) && !draftIds.has(tool.llmCallId)
  )
  const activeOrphanTools = orphanTools.filter(
    (tool) => tool.status === "streaming"
  )
  const orphanApprovals = approvalValues.filter(
    (approval) => !knownToolCallIds.has(approval.callId)
  )

  return (
    <>
      {messages.map((message) => {
        const isUser = message.role === "user"
        const text = message.text ?? JSON.stringify(message.json)
        const messageTools = message.toolCalls.flatMap((toolCall) => {
          const tool = tools[toolCall.callId]
          return tool ? [tool] : []
        })

        return (
          <div
            key={`${message.agentId}:${message.businessSeq}`}
            className="animate-in duration-200 fade-in-0 slide-in-from-bottom-2"
          >
            <Message from={isUser ? "user" : "assistant"}>
              {message.reasoning ? (
                <ReasoningDisclosure getThinkingMessage={thinkingMessage}>
                  {message.reasoning}
                </ReasoningDisclosure>
              ) : null}
              <MessageContent className="text-base! leading-6! group-[.is-user]:rounded-xl!">
                <MessageResponse>{text}</MessageResponse>
              </MessageContent>
              <ExecutionTraceGroup
                tools={messageTools}
                approvalsByCallId={approvalsByCallId}
                approvalSubmissions={approvalSubmissions}
                onApprovalDecision={onApprovalDecision}
              />
              <time
                dateTime={message.timestamp}
                className={
                  isUser
                    ? "self-end px-1 text-sm text-muted-foreground"
                    : "px-1 text-sm text-muted-foreground"
                }
              >
                {dateTimeFormat.format(new Date(message.timestamp))}
              </time>
            </Message>
          </div>
        )
      })}

      {Object.entries(drafts).map(([callId, draft]) => (
        <div
          key={callId}
          className="animate-in duration-200 fade-in-0 slide-in-from-bottom-2"
        >
          <Message from="assistant">
            {draft.reasoning ? (
              <ReasoningDisclosure
                isStreaming
                getThinkingMessage={thinkingMessage}
              >
                {draft.reasoning}
              </ReasoningDisclosure>
            ) : null}
            {draft.text ? (
              <MessageContent className="text-base! leading-6!">
                <MessageResponse>{draft.text}</MessageResponse>
              </MessageContent>
            ) : null}
            <ExecutionTraceGroup
              tools={toolValues.filter(
                (tool) =>
                  tool.llmCallId === callId &&
                  !stableToolCallIds.has(tool.callId)
              )}
              approvalsByCallId={approvalsByCallId}
              approvalSubmissions={approvalSubmissions}
              onApprovalDecision={onApprovalDecision}
            />
          </Message>
        </div>
      ))}

      {activeOrphanTools.length > 0 ? (
        <Message from="assistant">
          <ExecutionTraceGroup
            tools={activeOrphanTools}
            approvalsByCallId={approvalsByCallId}
            approvalSubmissions={approvalSubmissions}
            onApprovalDecision={onApprovalDecision}
          />
        </Message>
      ) : null}

      {orphanApprovals.map((approval) => (
        <Message key={approval.approvalId} from="assistant">
          <AgentApprovalCard
            approval={approval}
            submittingDecision={
              approvalSubmissions.get(approval.approvalId) ?? null
            }
            onDecision={(decision) =>
              onApprovalDecision(approval.approvalId, decision)
            }
          />
        </Message>
      ))}

      {error ? (
        <div className="animate-in duration-200 fade-in-0 slide-in-from-bottom-2">
          <Message from="assistant">
            <MessageContent className="text-base! leading-6!">
              <div className="rounded-xl border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">
                <p className="font-semibold">{t("chat.connectionFailed")}</p>
                {error.message ? (
                  <p className="mt-1 text-destructive/80">{error.message}</p>
                ) : null}
              </div>
            </MessageContent>
          </Message>
        </div>
      ) : null}
    </>
  )
}
