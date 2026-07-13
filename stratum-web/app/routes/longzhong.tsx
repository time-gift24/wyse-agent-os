"use client"

import { useEffect, useState } from "react"
import { HistoryIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { ChatWorkspace } from "~/components/stratum/chat-workspace"
import { cn } from "~/lib/utils"
import { RouteTransition } from "~/components/stratum/route-transition"
import { SiteNavbar } from "~/components/stratum/site-navbar"
import { Button } from "~/components/ui/button"

export default function Longzhong() {
  const { t } = useTranslation()
  const [historyOpen, setHistoryOpen] = useState(false)

  // 进入页面后 250ms 自动展开 history drawer，与 navbar timeline 同步
  useEffect(() => {
    const reduceMotion = window.matchMedia(
      "(prefers-reduced-motion: reduce)"
    ).matches
    const timer = setTimeout(
      () => setHistoryOpen(true),
      reduceMotion ? 0 : 250
    )
    return () => clearTimeout(timer)
  }, [])

  return (
    <RouteTransition>
      <main>
        <SiteNavbar
          activeSection="longzhong"
          leftSlot={
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("chat.history.title")}
              aria-expanded={historyOpen}
              aria-controls="chat-history-drawer"
              onClick={() => setHistoryOpen((open) => !open)}
              className={cn(
                "text-muted-foreground hover:bg-stratum-paper/60 hover:text-foreground",
                historyOpen && "text-stratum-action"
              )}
            >
              <HistoryIcon className="size-4" aria-hidden="true" />
            </Button>
          }
        />
        <ChatWorkspace
          historyOpen={historyOpen}
          onHistoryOpenChange={setHistoryOpen}
        />
      </main>
    </RouteTransition>
  )
}
