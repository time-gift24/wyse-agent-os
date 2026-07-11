import type { KeyboardEventHandler, ReactNode, Ref } from "react"

import { SendIcon } from "lucide-react"

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
    <div className="rounded-xl border border-border bg-card shadow-[0_12px_28px_-24px_rgb(43_48_51/0.55)]">
      <div className="px-3.5 pt-3">
        <p className="text-sm font-medium text-foreground">{label}</p>
        <p className="mt-0.5 text-xs/relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      <Textarea
        ref={inputRef}
        aria-label={label}
        className="min-h-20 resize-none border-0 bg-transparent px-3.5 pt-2.5 shadow-none focus-visible:ring-0"
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={onKeyDown}
        placeholder={placeholder}
        rows={3}
        value={value}
      />
      <div className="flex items-center justify-between gap-3 border-t border-border/70 px-3 py-2">
        <div className="min-w-0 text-[0.625rem] text-muted-foreground">{footer}</div>
        <Button
          type="button"
          size="icon-sm"
          aria-label="Send message"
          disabled={disabled || value.trim() === ""}
          onClick={onSubmit}
        >
          <SendIcon aria-hidden="true" />
        </Button>
      </div>
    </div>
  )
}
