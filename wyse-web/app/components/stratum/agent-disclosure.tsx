import type { ComponentProps, ReactNode } from "react"
import { useState } from "react"
import { ChevronDownIcon } from "lucide-react"

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "~/components/ui/collapsible"
import { cn } from "~/lib/utils"

type AgentDisclosureProps = {
  icon: ReactNode
  label: ReactNode
  children: ReactNode
  className?: string
}

export function AgentDisclosure({
  icon,
  label,
  children,
  className,
}: AgentDisclosureProps) {
  const [open, setOpen] = useState(false)

  return (
    <Collapsible
      className={cn("not-prose mb-4", className)}
      onOpenChange={setOpen}
      open={open}
    >
      <CollapsibleTrigger
        data-slot="agent-disclosure-trigger"
        className="flex w-full items-center gap-2 text-left text-sm text-muted-foreground transition-colors hover:text-foreground"
      >
        {icon}
        {label}
        <ChevronDownIcon
          aria-hidden="true"
          className={cn(
            "size-4 transition-transform",
            open ? "rotate-180" : "rotate-0"
          )}
        />
      </CollapsibleTrigger>
      {children}
    </Collapsible>
  )
}

export function AgentDisclosureContent({
  className,
  keepMounted = true,
  ...props
}: ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      className={cn(
        "mt-4 text-sm text-muted-foreground outline-none",
        "data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:slide-out-to-top-2 data-[state=open]:animate-in data-[state=open]:slide-in-from-top-2",
        className
      )}
      keepMounted={keepMounted}
      {...props}
    />
  )
}
