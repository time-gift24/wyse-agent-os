import { useTranslation } from "react-i18next"

import { Button } from "~/components/ui/button"
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "~/components/ui/card"
import type { ApprovalRequest } from "~/features/agent-conversation/types"

type AgentApprovalCardProps = {
  approval: ApprovalRequest
  submitting: boolean
  onDecision(decision: "approve" | "reject"): void
}

export function AgentApprovalCard({
  approval,
  submitting,
  onDecision,
}: AgentApprovalCardProps) {
  const { t } = useTranslation()

  return (
    <Card size="sm" className="w-full">
      <CardHeader>
        <CardTitle>{t("chat.approvalRequest")}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        <p>{approval.toolName}</p>
        <p>
          {t("chat.toolKind")}: {approval.toolKind}
        </p>
        <p>
          {t("chat.dangerLevel")}: {approval.dangerLevel}
        </p>
        <pre className="max-h-48 overflow-auto rounded-md bg-muted p-2 text-[0.625rem]">
          {JSON.stringify(approval.arguments, null, 2)}
        </pre>
      </CardContent>
      <CardFooter className="justify-end gap-2 border-t">
        <Button
          type="button"
          disabled={submitting}
          onClick={() => onDecision("reject")}
          variant="destructive"
        >
          {t("chat.reject")}
        </Button>
        <Button
          type="button"
          disabled={submitting}
          onClick={() => onDecision("approve")}
        >
          {t("chat.approve")}
        </Button>
      </CardFooter>
    </Card>
  )
}
