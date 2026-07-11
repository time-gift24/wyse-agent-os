import type { HTMLAttributes, ReactNode } from "react"

import { cn } from "~/lib/utils"

type MessageProps = HTMLAttributes<HTMLDivElement> & {
  from: "user" | "assistant" | "tool"
}

export function AiMessage({ from, className, ...props }: MessageProps) {
  return (
    <article
      data-slot="ai-message"
      data-from={from}
      className={cn(
        "group/ai-message flex w-full min-w-0 flex-col gap-1.5",
        from === "user" ? "items-end" : "items-start",
        className
      )}
      {...props}
    />
  )
}

export function AiMessageHeader({ children }: { children: ReactNode }) {
  return (
    <p className="px-1 text-[0.625rem] font-medium tracking-wide text-muted-foreground">
      {children}
    </p>
  )
}

export function AiMessageContent({
  from,
  className,
  ...props
}: HTMLAttributes<HTMLDivElement> & { from: MessageProps["from"] }) {
  return (
    <div
      data-slot="ai-message-content"
      className={cn(
        "max-w-[min(44rem,92%)] whitespace-pre-wrap break-words text-sm/relaxed",
        from === "user"
          ? "rounded-[14px] rounded-tr-sm border border-border/70 bg-secondary px-3.5 py-2.5 text-secondary-foreground"
          : "w-full border-l-2 border-primary/25 py-0.5 pl-3 text-foreground",
        className
      )}
      {...props}
    />
  )
}

export function AiStreamingMark() {
  return (
    <span
      aria-label="Streaming"
      className="mr-1 inline-block size-1.5 rounded-full bg-primary align-middle motion-safe:animate-pulse"
    />
  )
}
