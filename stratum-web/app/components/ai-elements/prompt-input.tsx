"use client"

import type {
  ComponentProps,
  HTMLAttributes,
  KeyboardEventHandler,
  ReactNode,
} from "react"
import { useCallback, useState } from "react"

import {
  InputGroup,
  InputGroupAddon,
  InputGroupButton,
  InputGroupTextarea,
} from "~/components/ui/input-group"
import { cn } from "~/lib/utils"

export function PromptInput({
  className,
  children,
  ...props
}: ComponentProps<"form">) {
  return (
    <form
      data-slot="prompt-input"
      className={cn("w-full", className)}
      {...props}
    >
      <InputGroup className="overflow-hidden rounded-[2rem] border-border/90 bg-card/95 shadow-[0_18px_45px_-35px_rgb(43_48_51/0.9)] backdrop-blur-sm">
        {children}
      </InputGroup>
    </form>
  )
}

export function PromptInputBody({
  className,
  ...props
}: HTMLAttributes<HTMLDivElement>) {
  return <div className={cn("contents", className)} {...props} />
}

export function PromptInputTextarea({
  onKeyDown,
  className,
  placeholder = "What would you like to know?",
  ...props
}: ComponentProps<typeof InputGroupTextarea>) {
  const [isComposing, setIsComposing] = useState(false)
  const handleKeyDown: KeyboardEventHandler<HTMLTextAreaElement> = useCallback(
    (event) => {
      onKeyDown?.(event)
      if (event.defaultPrevented || event.key !== "Enter") return
      if (isComposing || event.nativeEvent.isComposing || event.shiftKey) return
      event.preventDefault()
      const submit = event.currentTarget.form?.querySelector<HTMLButtonElement>(
        'button[type="submit"]'
      )
      if (!submit?.disabled) event.currentTarget.form?.requestSubmit()
    },
    [isComposing, onKeyDown]
  )

  return (
    <InputGroupTextarea
      className={cn(
        "field-sizing-content max-h-32 min-h-[2.5rem] px-5 py-2",
        className
      )}
      name="message"
      onCompositionEnd={() => setIsComposing(false)}
      onCompositionStart={() => setIsComposing(true)}
      onKeyDown={handleKeyDown}
      placeholder={placeholder}
      rows={1}
      {...props}
    />
  )
}

export function PromptInputFooter({
  className,
  ...props
}: Omit<ComponentProps<typeof InputGroupAddon>, "align">) {
  return (
    <InputGroupAddon
      align="block-end"
      className={cn("justify-between gap-1 px-5 pt-1 pb-3", className)}
      {...props}
    />
  )
}

export function PromptInputTools({
  className,
  ...props
}: HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("flex min-w-0 flex-1 items-center gap-1", className)}
      {...props}
    />
  )
}

export type PromptInputButtonProps = ComponentProps<typeof InputGroupButton>

export function PromptInputButton({
  variant = "ghost",
  size,
  className,
  ...props
}: PromptInputButtonProps) {
  return (
    <InputGroupButton
      className={className}
      size={size ?? "sm"}
      type="button"
      variant={variant}
      {...props}
    />
  )
}

export type PromptInputStatus = "submitted" | "streaming" | "ready" | "error"

export type PromptInputSubmitProps = ComponentProps<typeof InputGroupButton> & {
  status?: PromptInputStatus
  children?: ReactNode
}

export function PromptInputSubmit({
  status = "ready",
  className,
  children,
  ...props
}: PromptInputSubmitProps) {
  return (
    <InputGroupButton
      aria-label={status === "ready" ? "Submit" : "Stop"}
      className={cn("size-10 rounded-full", className)}
      size="icon-sm"
      type="submit"
      variant="default"
      {...props}
    >
      {children}
    </InputGroupButton>
  )
}
