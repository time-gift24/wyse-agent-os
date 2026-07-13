"use client"

import {
  CheckCircleIcon,
  ChevronDownIcon,
  CircleIcon,
  ClockIcon,
  WrenchIcon,
  XCircleIcon,
} from "lucide-react"
import type { ComponentProps, ReactNode } from "react"

import { Badge } from "~/components/ui/badge"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "~/components/ui/collapsible"
import { cn } from "~/lib/utils"

export type ToolStatus =
  | "pending"
  | "running"
  | "completed"
  | "denied"
  | "error"

export type ToolProps = ComponentProps<typeof Collapsible>

export const Tool = ({ className, ...props }: ToolProps) => (
  <Collapsible
    className={cn("group not-prose mb-4 w-full rounded-md border", className)}
    {...props}
  />
)

type ToolHeaderProps = ComponentProps<typeof CollapsibleTrigger> & {
  title: string
  status: ToolStatus
  statusLabel: string
}

const statusIcons: Record<ToolStatus, ReactNode> = {
  pending: <CircleIcon className="size-4" />,
  running: <ClockIcon className="size-4 animate-pulse" />,
  completed: <CheckCircleIcon className="size-4 text-green-600" />,
  denied: <XCircleIcon className="size-4 text-orange-600" />,
  error: <XCircleIcon className="size-4 text-red-600" />,
}

export const ToolHeader = ({
  className,
  title,
  status,
  statusLabel,
  ...props
}: ToolHeaderProps) => (
  <CollapsibleTrigger
    className={cn(
      "flex w-full items-center justify-between gap-4 p-3",
      className
    )}
    {...props}
  >
    <span className="flex min-w-0 items-center gap-2">
      <WrenchIcon className="size-4 shrink-0 text-muted-foreground" />
      <span className="truncate text-sm font-medium">{title}</span>
      <Badge className="gap-1.5 rounded-full text-xs" variant="secondary">
        {statusIcons[status]}
        {statusLabel}
      </Badge>
    </span>
    <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground transition-transform group-data-[state=open]:rotate-180" />
  </CollapsibleTrigger>
)

export type ToolContentProps = ComponentProps<typeof CollapsibleContent>

export const ToolContent = ({ className, ...props }: ToolContentProps) => (
  <CollapsibleContent
    className={cn(
      "space-y-4 p-4 text-popover-foreground outline-none",
      "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:slide-out-to-top-2 data-[state=open]:animate-in data-[state=open]:slide-in-from-top-2",
      className
    )}
    {...props}
  />
)

Tool.displayName = "Tool"
ToolHeader.displayName = "ToolHeader"
ToolContent.displayName = "ToolContent"
