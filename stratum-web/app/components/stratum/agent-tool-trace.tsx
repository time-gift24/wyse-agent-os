import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { CheckCircle2Icon, CircleDashedIcon, CircleXIcon } from "lucide-react"

import { Shimmer } from "~/components/ai-elements/shimmer"
import {
  AgentDisclosure,
  AgentDisclosureContent,
} from "~/components/stratum/agent-disclosure"
import type { ToolProgress } from "~/features/agent-conversation/types"
import { cn } from "~/lib/utils"
import { humanizeToolName } from "./agent-execution-presentation"

type ToolTraceRowProps = {
  tool: ToolProgress
  children?: ReactNode
}

function technicalValue(value: unknown): string {
  if (typeof value === "string") return value
  return JSON.stringify(value, null, 2)
}

export function ToolTraceRow({ tool, children }: ToolTraceRowProps) {
  const { t } = useTranslation()
  const toolName = humanizeToolName(tool.name, t("chat.unknownTool"))
  const statusCopy = t(`chat.toolTrace.status.${tool.status}`, {
    tool: toolName,
  })
  const StatusIcon =
    tool.status === "finished"
      ? CheckCircle2Icon
      : tool.status === "failed"
        ? CircleXIcon
        : CircleDashedIcon

  return (
    <div>
      <AgentDisclosure
        icon={
          <StatusIcon
            aria-hidden="true"
            className={cn(
              "size-4 shrink-0",
              tool.status === "finished" && "text-stratum-success",
              tool.status === "streaming" && "text-stratum-info",
              tool.status === "failed" && "text-destructive"
            )}
          />
        }
        label={
          tool.status === "streaming" ? (
            <Shimmer as="span" duration={1.4}>
              {statusCopy}
            </Shimmer>
          ) : (
            statusCopy
          )
        }
      >
        <AgentDisclosureContent className="grid gap-3">
          {tool.argumentsText ? (
            <div>
              <p className="mb-1 text-sm text-muted-foreground">
                {t("chat.toolTrace.input")}
              </p>
              <pre className="max-h-56 overflow-auto rounded-xl border border-stratum-line bg-muted/50 px-3 py-2 font-mono text-sm leading-relaxed break-words whitespace-pre-wrap text-foreground/80">
                {tool.argumentsText}
              </pre>
            </div>
          ) : null}
          {tool.result !== null ? (
            <div>
              <p className="mb-1 text-sm text-muted-foreground">
                {t("chat.toolTrace.rawResult")}
              </p>
              <pre className="max-h-56 overflow-auto rounded-xl border border-stratum-line bg-muted/50 px-3 py-2 font-mono text-sm leading-relaxed break-words whitespace-pre-wrap text-foreground/80">
                {technicalValue(tool.result)}
              </pre>
            </div>
          ) : null}
          {tool.errorText ? (
            <div>
              <p className="mb-1 text-sm text-muted-foreground">
                {t("chat.toolTrace.error")}
              </p>
              <pre className="max-h-56 overflow-auto rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2 font-mono text-sm leading-relaxed break-words whitespace-pre-wrap text-destructive">
                {tool.errorText}
              </pre>
            </div>
          ) : null}
        </AgentDisclosureContent>
      </AgentDisclosure>
      {children}
    </div>
  )
}
