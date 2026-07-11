import type { ComponentProps } from "react"

import { ArrowUpIcon } from "lucide-react"

import { Button } from "~/components/ui/button"
import { Textarea } from "~/components/ui/textarea"
import { cn } from "~/lib/utils"

export function PromptInput({
  className,
  ...props
}: ComponentProps<"form">) {
  return (
    <form
      data-slot="prompt-input"
      className={cn(
        "overflow-hidden rounded-[2rem] border border-border/90 bg-card/95 shadow-[0_18px_45px_-35px_rgb(43_48_51/0.9)] backdrop-blur-sm",
        className
      )}
      {...props}
    />
  )
}

export function PromptInputBody({
  className,
  ...props
}: ComponentProps<"div">) {
  return <div data-slot="prompt-input-body" className={className} {...props} />
}

export function PromptInputTextarea({
  className,
  ...props
}: ComponentProps<typeof Textarea>) {
  return (
    <Textarea
      className={cn(
        "min-h-36 resize-none border-0 bg-transparent px-5 pt-5 pb-1 shadow-none focus-visible:ring-0",
        className
      )}
      rows={4}
      {...props}
    />
  )
}

export function PromptInputFooter({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      data-slot="prompt-input-footer"
      className={cn(
        "flex items-center justify-between gap-3 px-5 pt-1 pb-4",
        className
      )}
      {...props}
    />
  )
}

export function PromptInputTools({
  className,
  ...props
}: ComponentProps<"div">) {
  return (
    <div
      data-slot="prompt-input-tools"
      className={cn("min-w-0 flex-1 text-[0.625rem] text-muted-foreground", className)}
      {...props}
    />
  )
}

export function PromptInputSubmit({
  ariaLabel,
  className,
  ...props
}: ComponentProps<typeof Button> & { ariaLabel: string }) {
  return (
    <Button
      type="submit"
      size="icon"
      aria-label={ariaLabel}
      className={cn("size-10 rounded-full", className)}
      {...props}
    >
      <ArrowUpIcon aria-hidden="true" />
    </Button>
  )
}
