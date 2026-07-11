import type { ReactNode } from "react"

import { cn } from "~/lib/utils"

export function AiTool({
  name,
  status,
  children,
}: {
  name: string
  status: string
  children: ReactNode
}) {
  return (
    <details className="group/ai-tool w-full rounded-md border border-border/70 bg-card/65">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-3 py-2 text-xs marker:hidden">
        <span className="font-medium text-foreground">{name}</span>
        <span className="rounded-full bg-muted px-2 py-0.5 text-[0.625rem] font-medium text-muted-foreground">
          {status}
        </span>
      </summary>
      <div className="border-t border-border/70 px-3 py-2 text-xs/relaxed text-muted-foreground">
        {children}
      </div>
    </details>
  )
}
