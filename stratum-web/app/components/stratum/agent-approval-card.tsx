import { useTranslation } from "react-i18next"
import { ShieldCheckIcon } from "lucide-react"

import { Button } from "~/components/ui/button"
import type { ApprovalRequest } from "~/features/agent-conversation/types"
import type { ApprovalDecision } from "./agent-approval-submissions"
import {
  approvalResource,
  humanizeToolName,
} from "./agent-execution-presentation"

type AgentApprovalCardProps = {
  approval: ApprovalRequest
  submittingDecision: ApprovalDecision | null
  onDecision(decision: "approve" | "reject"): void
}

export function AgentApprovalCard({
  approval,
  submittingDecision,
  onDecision,
}: AgentApprovalCardProps) {
  const { t } = useTranslation()
  const toolName = humanizeToolName(approval.toolName, t("chat.unknownTool"))
  const agentName = approval.agentName || t("chat.assistant")
  const resource = approvalResource(approval.arguments)
  const title = resource
    ? t("chat.approval.titleWithResource", {
        agent: agentName,
        tool: toolName,
        resource,
      })
    : t("chat.approval.title", { agent: agentName, tool: toolName })
  const submitting = submittingDecision !== null
  const titleId = `approval-${approval.approvalId}-title`

  return (
    <section
      aria-labelledby={titleId}
      className="mt-2 rounded-xl bg-primary/[0.055] px-4 py-4"
    >
      <div className="flex items-start gap-3">
        <ShieldCheckIcon
          aria-hidden="true"
          className="mt-0.5 size-4 shrink-0 text-primary"
        />
        <div className="min-w-0 flex-1">
          <h3 id={titleId} className="text-sm font-semibold text-foreground">
            {title}
          </h3>
        </div>
      </div>

      <div className="mt-4 flex justify-end gap-2">
        <Button
          type="button"
          className="h-9 px-3"
          disabled={submitting}
          onClick={() => onDecision("reject")}
          variant="ghost"
        >
          {submittingDecision === "reject"
            ? t("chat.approval.declining")
            : t("chat.reject")}
        </Button>
        <Button
          type="button"
          className="h-9 px-3"
          disabled={submitting}
          onClick={() => onDecision("approve")}
        >
          {submittingDecision === "approve"
            ? t("chat.approval.allowing")
            : t("chat.approve")}
        </Button>
      </div>
      <span aria-live="polite" className="sr-only">
        {submittingDecision === "approve"
          ? t("chat.approval.allowing")
          : submittingDecision === "reject"
            ? t("chat.approval.declining")
            : ""}
      </span>
    </section>
  )
}
