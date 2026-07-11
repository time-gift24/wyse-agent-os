import type { KeyboardEventHandler, ReactNode, Ref } from "react"

import { ArrowUpIcon, CornerDownLeftIcon } from "lucide-react"

import { Button } from "~/components/ui/button"
import { Textarea } from "~/components/ui/textarea"
import { cn } from "~/lib/utils"

export function AiPromptInput({
  inputRef,
  value,
  disabled,
  label,
  description,
  placeholder,
  shortcutHint,
  onChange,
  onSubmit,
  footer,
}: {
  inputRef: Ref<HTMLTextAreaElement>
  value: string
  disabled: boolean
  label: string
  description: string
  placeholder: string
  shortcutHint: string
  onChange(value: string): void
  onSubmit(): void
  footer: ReactNode
}) {
  const onKeyDown: KeyboardEventHandler<HTMLTextAreaElement> = (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault()
      onSubmit()
    }
  }

  return (
    <form
      data-slot="prompt-input"
      className="overflow-hidden rounded-2xl border border-border/90 bg-card/95 shadow-[0_18px_45px_-35px_rgb(43_48_51/0.9)] backdrop-blur-sm"
      onSubmit={(event) => {
        event.preventDefault()
        onSubmit()
      }}
    >
      <div
        data-slot="prompt-input-header"
        className="flex items-start justify-between gap-4 border-b border-border/70 px-4 py-3"
      >
        <div className="min-w-0">
          <p className="text-sm font-medium text-foreground">{label}</p>
          <p className="mt-0.5 text-xs/relaxed text-muted-foreground">
            {description}
          </p>
        </div>
        <span className="shrink-0 rounded-full border border-primary/25 bg-primary/8 px-2 py-0.5 text-[0.625rem] font-medium tracking-wide text-primary">
          WYSE
        </span>
      </div>
      <div data-slot="prompt-input-body">
        <Textarea
          ref={inputRef}
          aria-label={label}
          className="min-h-28 resize-none border-0 bg-transparent px-4 py-3 shadow-none focus-visible:ring-0"
          disabled={disabled}
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={onKeyDown}
          placeholder={placeholder}
          rows={4}
          value={value}
        />
      </div>
      <div
        data-slot="prompt-input-footer"
        className="flex items-center justify-between gap-3 border-t border-border/70 px-3 py-2.5"
      >
        <div data-slot="prompt-input-tools" className="flex min-w-0 items-center gap-2">
          <span className="inline-flex shrink-0 items-center gap-1 rounded-md bg-muted px-2 py-1 text-[0.625rem] text-muted-foreground">
            <CornerDownLeftIcon aria-hidden="true" className="size-3" />
            {shortcutHint}
          </span>
          <div className="min-w-0 text-[0.625rem] text-muted-foreground">
            {footer}
          </div>
        </div>
        <Button
          type="submit"
          size="icon"
          aria-label="Send message"
          className={cn("size-8 rounded-full", value.trim() === "" && "bg-muted text-muted-foreground hover:bg-muted")}
          disabled={disabled || value.trim() === ""}
        >
          <ArrowUpIcon aria-hidden="true" />
        </Button>
      </div>
    </form>
  )
}
