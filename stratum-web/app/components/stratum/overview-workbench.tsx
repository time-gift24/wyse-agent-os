"use client"

import {
  ArrowRightIcon,
  BotIcon,
  Clock3Icon,
  CpuIcon,
  RefreshCwIcon,
} from "lucide-react"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"

import {
  useProductWorkbench,
  type WorkbenchResource,
} from "~/components/stratum/product-shell"
import { Button, buttonVariants } from "~/components/ui/button"
import type { AgentTemplateView, ModelDescriptor } from "~/lib/model-config"
import { modelDisplayName } from "~/lib/model-config"
import { formatRelativeTime } from "~/lib/recent-agents"
import { cn } from "~/lib/utils"

function ResourceSkeleton({ rows = 3 }: { rows?: number }) {
  return (
    <div className="space-y-1" aria-hidden="true">
      {Array.from({ length: rows }, (_, index) => (
        <div
          key={index}
          className="flex min-h-14 items-center gap-3 border-t border-stratum-line px-1 first:border-t-0"
        >
          <span className="size-9 animate-pulse rounded-lg bg-stratum-paper-soft motion-reduce:animate-none" />
          <span className="h-4 w-2/5 animate-pulse rounded bg-stratum-paper-soft motion-reduce:animate-none" />
        </div>
      ))}
    </div>
  )
}

function ResourceFailure({
  message,
  onRetry,
}: {
  message: string
  onRetry(): void
}) {
  const { t } = useTranslation()
  return (
    <div className="rounded-xl bg-destructive/8 px-4 py-4 text-sm text-foreground">
      <p>{message}</p>
      <Button
        type="button"
        variant="outline"
        onClick={onRetry}
        className="mt-3 min-h-11 rounded-lg border-stratum-line-strong px-3 text-sm"
      >
        <RefreshCwIcon className="size-4" aria-hidden="true" />
        {t("overview.retry")}
      </Button>
    </div>
  )
}

function MetadataSummary({
  resource,
  icon: Icon,
  label,
  emptyLabel,
}: {
  resource: WorkbenchResource<AgentTemplateView | ModelDescriptor>
  icon: typeof BotIcon
  label: string
  emptyLabel: string
}) {
  const value =
    resource.phase === "loading"
      ? "…"
      : resource.phase === "error"
        ? "-"
        : String(resource.items.length)
  return (
    <div className="flex min-w-0 items-center gap-3 py-3">
      <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-stratum-paper-soft text-muted-foreground">
        <Icon className="size-[18px] stroke-[1.8]" aria-hidden="true" />
      </span>
      <div className="min-w-0">
        <p className="text-[13px] text-muted-foreground">{label}</p>
        <p className="truncate text-base font-semibold text-foreground">
          {resource.phase === "empty" ? emptyLabel : value}
        </p>
      </div>
    </div>
  )
}

export function OverviewWorkbench() {
  const { t, i18n } = useTranslation()
  const { templates, models, recentAgents, refreshTemplates, refreshModels } =
    useProductWorkbench()
  const language = i18n.resolvedLanguage ?? "en"

  return (
    <div className="stratum-workbench-width mx-auto px-4 pb-12 sm:px-6 lg:px-8">
      <section className="overflow-hidden rounded-2xl border border-stratum-line bg-stratum-paper-wash">
        <div className="px-6 pt-7 pb-6 sm:px-8 sm:pt-9 lg:px-10 lg:pt-10">
          <h1 className="type-hero max-w-[16ch] text-balance text-foreground">
            {t("overview.title")}
          </h1>
          <p className="mt-4 max-w-[62ch] text-base leading-7 text-pretty text-muted-foreground">
            {t("overview.description")}
          </p>
          <Link
            to="/longzhong?new=1"
            onClick={() => {
              document.documentElement.dataset.navigationDirection = "forward"
            }}
            className={cn(
              buttonVariants({ size: "lg" }),
              "mt-7 min-h-11 rounded-lg bg-primary px-4 text-sm font-semibold text-primary-foreground hover:bg-stratum-action-hover"
            )}
          >
            {t("overview.startConversation")}
            <ArrowRightIcon
              data-icon="inline-end"
              className="size-4"
              aria-hidden="true"
            />
          </Link>
        </div>

        <div className="grid border-t border-stratum-line px-6 sm:grid-cols-2 sm:px-8 lg:px-10">
          <MetadataSummary
            resource={models}
            icon={CpuIcon}
            label={t("overview.models")}
            emptyLabel={t("overview.noneAvailable")}
          />
          <div className="border-t border-stratum-line sm:border-t-0 sm:border-l sm:pl-6">
            <MetadataSummary
              resource={templates}
              icon={BotIcon}
              label={t("overview.agentTemplates")}
              emptyLabel={t("overview.noneAvailable")}
            />
          </div>
        </div>
      </section>

      <div className="mt-8 grid gap-8 lg:grid-cols-[minmax(0,1.55fr)_minmax(280px,0.75fr)]">
        <section aria-labelledby="agent-templates-heading">
          <div className="mb-3 flex items-end justify-between gap-4">
            <div>
              <h2
                id="agent-templates-heading"
                className="text-xl font-semibold tracking-[-0.01em] text-foreground"
              >
                {t("overview.chooseAgent")}
              </h2>
              <p className="mt-1 text-sm text-muted-foreground">
                {t("overview.chooseAgentDescription")}
              </p>
            </div>
          </div>

          <div className="rounded-xl border border-stratum-line bg-stratum-paper px-4 py-2 sm:px-5">
            {templates.phase === "loading" ? (
              <ResourceSkeleton />
            ) : templates.phase === "error" ? (
              <ResourceFailure
                message={t("overview.templatesError")}
                onRetry={() => void refreshTemplates()}
              />
            ) : templates.phase === "empty" ? (
              <div className="py-8 text-center">
                <BotIcon
                  className="mx-auto size-6 text-muted-foreground"
                  aria-hidden="true"
                />
                <p className="mt-3 text-sm font-semibold text-foreground">
                  {t("overview.noTemplates")}
                </p>
                <p className="mt-1 text-sm text-muted-foreground">
                  {t("overview.noTemplatesDescription")}
                </p>
              </div>
            ) : (
              <ul>
                {templates.items.slice(0, 6).map((template) => {
                  const model = modelDisplayName(template.model_config.model)
                  return (
                    <li
                      key={template.agent_name}
                      className="border-t border-stratum-line first:border-t-0"
                    >
                      <Link
                        to={`/longzhong?template=${encodeURIComponent(template.agent_name)}`}
                        className="group flex min-h-[68px] items-center gap-3 rounded-lg px-1 py-3 text-left transition-colors duration-200 hover:bg-stratum-paper-soft sm:px-3"
                      >
                        <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-stratum-paper-soft text-muted-foreground transition-colors duration-200 group-hover:text-foreground">
                          <BotIcon
                            className="size-[18px] stroke-[1.8]"
                            aria-hidden="true"
                          />
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-base font-semibold text-foreground">
                            {template.agent_name}
                          </span>
                          <span className="mt-0.5 block truncate text-[13px] text-muted-foreground">
                            {model.provider
                              ? `${model.provider} / ${model.model}`
                              : model.model}
                          </span>
                        </span>
                        <ArrowRightIcon
                          className="size-4 shrink-0 text-muted-foreground transition-transform duration-200 group-hover:translate-x-0.5 group-hover:text-foreground motion-reduce:transition-none"
                          aria-hidden="true"
                        />
                      </Link>
                    </li>
                  )
                })}
              </ul>
            )}
          </div>

          {models.phase === "error" ? (
            <div className="mt-4">
              <ResourceFailure
                message={t("overview.modelsError")}
                onRetry={() => void refreshModels()}
              />
            </div>
          ) : null}
        </section>

        <section aria-labelledby="recent-conversations-heading">
          <h2
            id="recent-conversations-heading"
            className="text-xl font-semibold tracking-[-0.01em] text-foreground"
          >
            {t("overview.recentConversations")}
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {t("overview.recentDescription")}
          </p>

          <div className="mt-3 rounded-xl border border-stratum-line bg-stratum-paper px-4 py-2">
            {recentAgents.length === 0 ? (
              <div className="py-8 text-center">
                <Clock3Icon
                  className="mx-auto size-6 text-muted-foreground"
                  aria-hidden="true"
                />
                <p className="mt-3 text-sm font-semibold text-foreground">
                  {t("overview.noRecent")}
                </p>
                <p className="mt-1 text-sm leading-6 text-muted-foreground">
                  {t("overview.noRecentDescription")}
                </p>
              </div>
            ) : (
              <ul>
                {recentAgents.slice(0, 5).map((agent) => (
                  <li
                    key={agent.agentId}
                    className="border-t border-stratum-line first:border-t-0"
                  >
                    <Link
                      to={`/longzhong?agent=${encodeURIComponent(agent.agentId)}`}
                      className="group flex min-h-[60px] items-center gap-3 rounded-lg py-2 transition-colors duration-200 hover:bg-stratum-paper-soft"
                    >
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-semibold text-foreground">
                          {agent.title}
                        </span>
                        <span className="mt-0.5 block text-[13px] text-muted-foreground">
                          {formatRelativeTime(agent.lastOpenedAt, language)}
                        </span>
                      </span>
                      <ArrowRightIcon
                        className="size-4 shrink-0 text-muted-foreground transition-transform duration-200 group-hover:translate-x-0.5 motion-reduce:transition-none"
                        aria-hidden="true"
                      />
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </section>
      </div>
    </div>
  )
}
