"use client"

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"
import {
  AlertCircleIcon,
  Clock3Icon,
  HouseIcon,
  LoaderCircleIcon,
  MenuIcon,
  MessageSquareTextIcon,
  PlusIcon,
  Trash2Icon,
  UserPlusIcon,
  XIcon,
} from "lucide-react"
import { AnimatePresence, motion, useReducedMotion } from "motion/react"
import { Link, useLocation, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"

import { LanguageToggle } from "~/components/stratum/language-toggle"
import { StratumMark } from "~/components/stratum/stratum-mark"
import { ThemeToggle } from "~/components/stratum/theme-toggle"
import { Button } from "~/components/ui/button"
import type { AgentTemplateView, ModelDescriptor } from "~/lib/model-config"
import {
  formatRelativeTime,
  loadRecentAgents,
  rememberRecentAgent as rememberStoredRecentAgent,
  removeRecentAgent as removeStoredRecentAgent,
  type RecentAgent,
  type StorageLike,
} from "~/lib/recent-agents"
import {
  ApiError,
  createStratumApi,
  STRATUM_API_BASE_URL,
} from "~/lib/stratum-api"
import { cn } from "~/lib/utils"

type ResourcePhase = "loading" | "ready" | "empty" | "error"

export type WorkbenchResource<T> = {
  items: readonly T[]
  phase: ResourcePhase
  error: ApiError | null
}

type ProductWorkbenchContextValue = {
  templates: WorkbenchResource<AgentTemplateView>
  models: WorkbenchResource<ModelDescriptor>
  recentAgents: readonly RecentAgent[]
  activeAgentId: string | null
  missingAgentId: string | null
  metadataLoading: boolean
  metadataError: ApiError | null
  refreshTemplates(): Promise<void>
  refreshModels(): Promise<void>
  rememberRecentAgent(agent: RecentAgent): void
  removeRecentAgent(agentId: string): void
  setActiveAgentId(agentId: string | null): void
  setMissingAgentId(agentId: string | null): void
}

const ProductWorkbenchContext =
  createContext<ProductWorkbenchContextValue | null>(null)

const initialResource = <T,>(): WorkbenchResource<T> => ({
  items: [],
  phase: "loading",
  error: null,
})

export function useProductWorkbench(): ProductWorkbenchContextValue {
  const context = useContext(ProductWorkbenchContext)
  if (!context)
    throw new Error("useProductWorkbench must be used inside ProductShell")
  return context
}

function browserStorage(): StorageLike | undefined {
  if (typeof window === "undefined") return undefined
  try {
    return window.localStorage
  } catch {
    return undefined
  }
}

function toApiError(error: unknown): ApiError {
  if (error instanceof ApiError) return error
  return new ApiError(
    "metadata_failed",
    0,
    error instanceof Error ? error.message : "metadata request failed"
  )
}

type SidebarContentProps = {
  activeSection: "overview" | "longzhong"
  activeAgentId: string | null
  missingAgentId: string | null
  recentAgents: readonly RecentAgent[]
  onNavigate(): void
  onRemoveAgent(agentId: string): void
}

function SidebarContent({
  activeSection,
  activeAgentId,
  missingAgentId,
  recentAgents,
  onNavigate,
  onRemoveAgent,
}: SidebarContentProps) {
  const { t, i18n } = useTranslation()
  const language = i18n.resolvedLanguage ?? "en"
  const navigation = [
    {
      id: "overview" as const,
      label: t("nav.overview"),
      to: "/",
      icon: HouseIcon,
    },
    {
      id: "longzhong" as const,
      label: t("nav.longzhong"),
      to: "/longzhong",
      icon: MessageSquareTextIcon,
    },
  ]

  return (
    <div className="flex h-full min-h-0 flex-col p-5">
      <Link
        to="/"
        onClick={onNavigate}
        aria-label={`运筹 ${t("brand.home")}`}
        className="flex min-h-11 items-center gap-2 rounded-lg px-2 text-foreground"
      >
        <StratumMark animated={false} variant="compact" className="size-8" />
        <span className="font-heading text-base font-semibold">运筹</span>
      </Link>

      <nav aria-label={t("productShell.navigation")} className="mt-6 space-y-1">
        {navigation.map((item) => {
          const Icon = item.icon
          const active = activeSection === item.id
          return (
            <Link
              key={item.id}
              to={item.to}
              aria-current={active ? "page" : undefined}
              onClick={() => {
                document.documentElement.dataset.navigationDirection =
                  item.id === "longzhong" ? "forward" : "back"
                onNavigate()
              }}
              className={cn(
                "flex min-h-11 items-center gap-3 rounded-[9px] px-3 text-sm font-medium transition-colors duration-200",
                active
                  ? "bg-stratum-paper-soft text-foreground"
                  : "text-muted-foreground hover:bg-stratum-paper-soft/70 hover:text-foreground"
              )}
            >
              <Icon className="size-[17px] stroke-[1.8]" aria-hidden="true" />
              <span>{item.label}</span>
            </Link>
          )
        })}
      </nav>

      <div className="mt-7 flex min-h-0 flex-1 flex-col">
        <div className="flex items-center justify-between gap-3 px-3">
          <h2 className="text-sm font-semibold text-foreground">
            {t("productShell.recent")}
          </h2>
          <Link
            to="/longzhong?new=1"
            onClick={onNavigate}
            aria-label={t("productShell.newConversation")}
            title={t("productShell.newConversation")}
            className="grid size-11 place-items-center rounded-lg text-muted-foreground transition-colors duration-200 hover:bg-stratum-paper-soft hover:text-foreground"
          >
            <PlusIcon className="size-4 stroke-[1.8]" aria-hidden="true" />
          </Link>
        </div>

        <div className="mt-1 min-h-0 overflow-y-auto">
          {recentAgents.length === 0 ? (
            <p className="px-3 py-3 text-[13px] leading-5 text-muted-foreground">
              {t("productShell.noRecent")}
            </p>
          ) : (
            <ul className="space-y-1">
              {recentAgents.slice(0, 6).map((agent) => {
                const active = agent.agentId === activeAgentId
                const missing = agent.agentId === missingAgentId
                return (
                  <li
                    key={agent.agentId}
                    className="group flex items-center gap-1"
                  >
                    <Link
                      to={`/longzhong?agent=${encodeURIComponent(agent.agentId)}`}
                      aria-current={active ? "page" : undefined}
                      onClick={onNavigate}
                      className={cn(
                        "flex min-h-11 min-w-0 flex-1 items-center gap-2 rounded-[9px] px-3 transition-colors duration-200",
                        active
                          ? "bg-stratum-paper-soft text-foreground"
                          : "text-muted-foreground hover:bg-stratum-paper-soft/70 hover:text-foreground",
                        missing && "text-destructive"
                      )}
                    >
                      <Clock3Icon
                        className="size-4 shrink-0 stroke-[1.8]"
                        aria-hidden="true"
                      />
                      <span className="min-w-0 flex-1 truncate text-sm">
                        {agent.title}
                      </span>
                      <span className="shrink-0 text-[11px] text-muted-foreground">
                        {formatRelativeTime(agent.lastOpenedAt, language)}
                      </span>
                    </Link>
                    {missing ? (
                      <Button
                        type="button"
                        size="icon"
                        variant="ghost"
                        aria-label={t("chat.removeLocalEntry")}
                        title={t("chat.removeLocalEntry")}
                        onClick={() => onRemoveAgent(agent.agentId)}
                        className="size-11 text-destructive"
                      >
                        <Trash2Icon className="size-4" aria-hidden="true" />
                      </Button>
                    ) : null}
                  </li>
                )
              })}
            </ul>
          )}
        </div>
      </div>

      <div className="mt-5 border-t border-stratum-line pt-4">
        <div className="flex items-center justify-between gap-2">
          <LanguageToggle />
          <ThemeToggle />
        </div>
        <Button
          type="button"
          variant="outline"
          className="mt-3 min-h-11 w-full justify-center gap-2 rounded-[9px] border-stratum-line-strong text-sm"
        >
          <UserPlusIcon className="size-4" aria-hidden="true" />
          {t("actions.signUp")}
        </Button>
      </div>
    </div>
  )
}

export function ProductShell({ children }: { children: ReactNode }) {
  const { t } = useTranslation()
  const location = useLocation()
  const navigate = useNavigate()
  const reduceMotion = useReducedMotion()
  const activeSection = location.pathname.startsWith("/longzhong")
    ? "longzhong"
    : "overview"
  const [templates, setTemplates] =
    useState<WorkbenchResource<AgentTemplateView>>(initialResource)
  const [models, setModels] =
    useState<WorkbenchResource<ModelDescriptor>>(initialResource)
  const [recentAgents, setRecentAgents] = useState<readonly RecentAgent[]>([])
  const [activeAgentId, setActiveAgentId] = useState<string | null>(null)
  const [missingAgentId, setMissingAgentId] = useState<string | null>(null)
  const [menuOpen, setMenuOpen] = useState(false)
  const menuButtonRef = useRef<HTMLButtonElement>(null)
  const drawerRef = useRef<HTMLElement>(null)
  const mainRef = useRef<HTMLElement>(null)
  const previousPathRef = useRef(location.pathname)

  const refreshTemplates = useCallback(async () => {
    setTemplates((resource) => ({ ...resource, phase: "loading", error: null }))
    try {
      const items = await createStratumApi({
        baseUrl: STRATUM_API_BASE_URL,
      }).getAgentTemplates()
      setTemplates({
        items,
        phase: items.length === 0 ? "empty" : "ready",
        error: null,
      })
    } catch (error) {
      setTemplates({ items: [], phase: "error", error: toApiError(error) })
    }
  }, [])

  const refreshModels = useCallback(async () => {
    setModels((resource) => ({ ...resource, phase: "loading", error: null }))
    try {
      const items = await createStratumApi({
        baseUrl: STRATUM_API_BASE_URL,
      }).getModels()
      setModels({
        items,
        phase: items.length === 0 ? "empty" : "ready",
        error: null,
      })
    } catch (error) {
      setModels({ items: [], phase: "error", error: toApiError(error) })
    }
  }, [])

  useEffect(() => {
    void Promise.allSettled([refreshTemplates(), refreshModels()])
    const storage = browserStorage()
    if (storage) setRecentAgents(loadRecentAgents(storage))
  }, [refreshModels, refreshTemplates])

  const rememberRecentAgent = useCallback((agent: RecentAgent) => {
    const storage = browserStorage()
    if (storage) {
      rememberStoredRecentAgent(storage, agent)
      setRecentAgents(loadRecentAgents(storage))
      return
    }
    setRecentAgents((agents) => [
      agent,
      ...agents.filter((recentAgent) => recentAgent.agentId !== agent.agentId),
    ])
  }, [])

  const removeRecentAgent = useCallback(
    (agentId: string) => {
      const storage = browserStorage()
      if (storage) {
        removeStoredRecentAgent(storage, agentId)
        setRecentAgents(loadRecentAgents(storage))
      } else {
        setRecentAgents((agents) =>
          agents.filter((agent) => agent.agentId !== agentId)
        )
      }
      if (missingAgentId === agentId) setMissingAgentId(null)
      if (activeAgentId === agentId) {
        setActiveAgentId(null)
        navigate("/longzhong?new=1")
      }
    },
    [activeAgentId, missingAgentId, navigate]
  )

  useEffect(() => {
    if (!menuOpen) return
    const previouslyFocused = document.activeElement as HTMLElement | null
    const previousOverflow = document.body.style.overflow
    document.body.style.overflow = "hidden"
    const focusFrame = requestAnimationFrame(() => {
      drawerRef.current
        ?.querySelector<HTMLElement>("button:not(:disabled), a[href]")
        ?.focus()
    })
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault()
        setMenuOpen(false)
        return
      }
      if (event.key !== "Tab" || !drawerRef.current) return
      const focusable = Array.from(
        drawerRef.current.querySelectorAll<HTMLElement>(
          'button:not(:disabled), a[href], [tabindex]:not([tabindex="-1"])'
        )
      )
      if (focusable.length === 0) return
      const first = focusable[0]
      const last = focusable.at(-1)
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault()
        last?.focus()
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault()
        first?.focus()
      }
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => {
      cancelAnimationFrame(focusFrame)
      window.removeEventListener("keydown", handleKeyDown)
      document.body.style.overflow = previousOverflow
      if (previouslyFocused === menuButtonRef.current)
        menuButtonRef.current?.focus()
    }
  }, [menuOpen])

  useEffect(() => {
    if (previousPathRef.current === location.pathname) return
    previousPathRef.current = location.pathname
    const focusFrame = requestAnimationFrame(() => mainRef.current?.focus())
    return () => cancelAnimationFrame(focusFrame)
  }, [location.pathname])

  const status = useMemo(() => {
    if (templates.phase === "loading" || models.phase === "loading")
      return {
        kind: "loading" as const,
        label: t("productShell.status.loading"),
      }
    if (templates.phase === "error" || models.phase === "error")
      return {
        kind: "error" as const,
        label: t("productShell.status.error"),
      }
    if (templates.phase === "empty" || models.phase === "empty")
      return {
        kind: "incomplete" as const,
        label: t("productShell.status.incomplete"),
      }
    return { kind: "ready" as const, label: t("productShell.status.ready") }
  }, [models.phase, t, templates.phase])

  const contextValue = useMemo<ProductWorkbenchContextValue>(
    () => ({
      templates,
      models,
      recentAgents,
      activeAgentId,
      missingAgentId,
      metadataLoading:
        templates.phase === "loading" || models.phase === "loading",
      metadataError: templates.error ?? models.error,
      refreshTemplates,
      refreshModels,
      rememberRecentAgent,
      removeRecentAgent,
      setActiveAgentId,
      setMissingAgentId,
    }),
    [
      activeAgentId,
      missingAgentId,
      models,
      recentAgents,
      refreshModels,
      refreshTemplates,
      rememberRecentAgent,
      removeRecentAgent,
      templates,
    ]
  )

  const pageTitle =
    activeSection === "longzhong" ? t("nav.longzhong") : t("nav.overview")

  return (
    <ProductWorkbenchContext.Provider value={contextValue}>
      <div className="min-h-[100dvh] bg-background text-foreground">
        <a
          href="#main-content"
          className="fixed top-2 left-2 z-[60] -translate-y-20 rounded-lg bg-primary px-4 py-3 text-sm font-semibold text-primary-foreground focus:translate-y-0"
        >
          {t("productShell.skipToContent")}
        </a>

        <aside className="fixed top-4 bottom-4 left-4 z-30 hidden w-[285px] overflow-hidden rounded-[14px] border border-stratum-line bg-sidebar shadow-stratum-rail xl:block">
          <SidebarContent
            activeSection={activeSection}
            activeAgentId={activeAgentId}
            missingAgentId={missingAgentId}
            recentAgents={recentAgents}
            onNavigate={() => setMenuOpen(false)}
            onRemoveAgent={removeRecentAgent}
          />
        </aside>

        <header className="fixed top-4 right-4 left-4 z-40 flex h-[75px] items-center gap-3 rounded-2xl border border-stratum-line bg-stratum-paper-wash px-3 shadow-stratum-topbar sm:px-5 xl:left-[317px]">
          <Button
            ref={menuButtonRef}
            type="button"
            variant="ghost"
            size="icon"
            aria-label={t("productShell.openNavigation")}
            aria-expanded={menuOpen}
            aria-controls="product-navigation-drawer"
            onClick={() => setMenuOpen(true)}
            className="size-11 rounded-lg xl:hidden"
          >
            <MenuIcon className="size-5 stroke-[1.8]" aria-hidden="true" />
          </Button>

          <div className="min-w-0">
            <p className="truncate text-sm font-semibold text-foreground">
              {pageTitle}
            </p>
            <div
              className={cn(
                "mt-0.5 flex items-center gap-1.5 text-[13px]",
                status.kind === "ready"
                  ? "text-stratum-success"
                  : status.kind === "error"
                    ? "text-destructive"
                    : "text-muted-foreground"
              )}
              role="status"
            >
              {status.kind === "ready" ? (
                <span
                  className="size-2 shrink-0 rounded-full bg-stratum-success"
                  aria-hidden="true"
                />
              ) : status.kind === "loading" ? (
                <LoaderCircleIcon
                  className="size-3.5 shrink-0 animate-spin motion-reduce:animate-none"
                  aria-hidden="true"
                />
              ) : (
                <AlertCircleIcon
                  className="size-3.5 shrink-0"
                  aria-hidden="true"
                />
              )}
              <span className="truncate">{status.label}</span>
            </div>
          </div>

          <div className="ml-auto flex items-center gap-1">
            <LanguageToggle compact />
            <ThemeToggle />
          </div>
        </header>

        <AnimatePresence initial={false}>
          {menuOpen ? (
            <div className="fixed inset-0 z-50 xl:hidden">
              <motion.button
                type="button"
                aria-label={t("productShell.closeNavigation")}
                className="absolute inset-0 bg-stratum-ink/25"
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                transition={{ duration: reduceMotion ? 0 : 0.18 }}
                onClick={() => setMenuOpen(false)}
              />
              <motion.aside
                ref={drawerRef}
                id="product-navigation-drawer"
                role="dialog"
                aria-modal="true"
                aria-label={t("productShell.navigation")}
                className="relative h-full w-[min(285px,calc(100vw-48px))] overflow-hidden border-r border-stratum-line bg-sidebar shadow-stratum-drawer"
                initial={{ x: reduceMotion ? 0 : -24, opacity: 0 }}
                animate={{ x: 0, opacity: 1 }}
                exit={{ x: reduceMotion ? 0 : -24, opacity: 0 }}
                transition={{
                  duration: reduceMotion ? 0 : 0.2,
                  ease: [0.16, 1, 0.3, 1],
                }}
              >
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  aria-label={t("productShell.closeNavigation")}
                  onClick={() => setMenuOpen(false)}
                  className="absolute top-3 right-3 z-10 size-11 rounded-lg"
                >
                  <XIcon className="size-5 stroke-[1.8]" aria-hidden="true" />
                </Button>
                <SidebarContent
                  activeSection={activeSection}
                  activeAgentId={activeAgentId}
                  missingAgentId={missingAgentId}
                  recentAgents={recentAgents}
                  onNavigate={() => setMenuOpen(false)}
                  onRemoveAgent={removeRecentAgent}
                />
              </motion.aside>
            </div>
          ) : null}
        </AnimatePresence>

        <div className="min-h-[100dvh] pt-[107px] xl:pl-[317px]">
          <main
            ref={mainRef}
            id="main-content"
            tabIndex={-1}
            className="min-h-[calc(100dvh-107px)] outline-none"
          >
            {children}
          </main>
        </div>
      </div>
    </ProductWorkbenchContext.Provider>
  )
}
