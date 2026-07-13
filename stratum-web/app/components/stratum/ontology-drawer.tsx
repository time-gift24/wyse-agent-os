"use client"

import { useEffect, useRef, type ReactNode, type RefObject } from "react"
import { XIcon } from "lucide-react"

import { Button } from "~/components/ui/button"
import { cn } from "~/lib/utils"

type OntologyDrawerProps = {
  open: boolean
  side: "left" | "right"
  label: string
  closeLabel: string
  returnFocusRef: RefObject<HTMLButtonElement | null>
  children: ReactNode
  onOpenChange(open: boolean): void
}

export function OntologyDrawer({
  open,
  side,
  label,
  closeLabel,
  returnFocusRef,
  children,
  onOpenChange,
}: OntologyDrawerProps) {
  const dialogRef = useRef<HTMLDialogElement>(null)

  useEffect(() => {
    const dialog = dialogRef.current
    if (!dialog) return
    if (open && !dialog.open) dialog.showModal()
    if (!open && dialog.open) dialog.close()
  }, [open])

  return (
    <dialog
      ref={dialogRef}
      aria-label={label}
      className={cn(
        "fixed inset-y-0 m-0 h-dvh max-h-none w-[min(22rem,calc(100vw-2rem))] max-w-none border-0 bg-wyse-paper p-0 text-foreground backdrop:bg-foreground/25 lg:hidden",
        side === "left" ? "mr-auto" : "ml-auto"
      )}
      onClose={() => {
        onOpenChange(false)
        returnFocusRef.current?.focus()
      }}
      onCancel={(event) => {
        event.preventDefault()
        onOpenChange(false)
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onOpenChange(false)
      }}
    >
      <div className="flex h-full min-h-0 flex-col">
        <div className="flex items-center justify-between border-b border-wyse-line px-4 py-3">
          <h2 className="text-sm font-semibold">{label}</h2>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="size-11"
            aria-label={closeLabel}
            onClick={() => onOpenChange(false)}
          >
            <XIcon aria-hidden="true" />
          </Button>
        </div>
        <div className="min-h-0 flex-1">{children}</div>
      </div>
    </dialog>
  )
}
