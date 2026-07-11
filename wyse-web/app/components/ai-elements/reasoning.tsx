import type { ReactNode } from "react"

import { cn } from "~/lib/utils"

export function AiReasoning({
  children,
  streaming = false,
}: {
  children: ReactNode
  streaming?: boolean
}) {
  return (
    <details
      open={streaming}
      className="group/ai-reasoning mt-1 w-full max-w-[44rem] rounded-md border border-border/60 bg-muted/35 px-3 py-2 text-xs/relaxed text-muted-foreground"
    >
      <summary className="cursor-pointer list-none select-none font-medium text-foreground marker:hidden">
        <span className={cn(streaming && "motion-safe:animate-pulse")}>
          {streaming ? "Thinking" : "Reasoning"}
        </span>
      </summary>
      <div className="mt-2 whitespace-pre-wrap border-t border-border/50 pt-2">
        {children}
      </div>
    </details>
  )
}
