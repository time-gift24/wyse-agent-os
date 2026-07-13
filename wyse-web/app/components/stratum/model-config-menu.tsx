import { ChevronDownIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "~/components/ui/dropdown-menu"
import type { ComposerConfiguration } from "~/hooks/use-agent-conversation"
import {
  isModelConfigMenuDisabled,
  modelDisplayName,
  supportsThinkingControls,
} from "~/lib/model-config"

type ConfigurationMenuProps = {
  configuration: ComposerConfiguration
  commandPending: boolean
}

export function AgentConfigMenu({
  configuration,
  commandPending,
}: ConfigurationMenuProps) {
  const { t } = useTranslation()
  const triggerText = configuration.metadataLoading
    ? t("chat.composer.loadingConfiguration")
    : (configuration.agentName ?? t("chat.composer.selectAgent"))

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={triggerText}
        className="inline-flex h-8 max-w-36 min-w-0 flex-1 items-center gap-1 rounded-md px-2 text-xs text-muted-foreground transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 sm:flex-none"
        disabled={menuDisabled(configuration, commandPending)}
      >
        <span className="truncate">{triggerText}</span>
        <ChevronDownIcon className="size-3.5 shrink-0" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-56">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t("chat.composer.agent")}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={configuration.agentName ?? undefined}
            onValueChange={(agentName) => {
              const template = configuration.agentTemplates.find(
                (candidate) => candidate.agent_name === agentName
              )
              if (template) configuration.selectTemplate(template)
            }}
          >
            {configuration.agentTemplates.map((template) => (
              <DropdownMenuRadioItem
                key={template.agent_name}
                value={template.agent_name}
              >
                {template.agent_name}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

export function ModelConfigMenu({
  configuration,
  commandPending,
}: ConfigurationMenuProps) {
  const { t } = useTranslation()
  const selected = configuration.selectedModelConfig
  const selectedDescriptor = configuration.models.find(
    (descriptor) => descriptor.model === selected?.model
  )
  const triggerText = configuration.metadataLoading
    ? t("chat.composer.loadingConfiguration")
    : selected === null
      ? t("chat.composer.selectAgent")
      : formatModelLabel(selected.model)

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={triggerText}
        className="inline-flex h-8 max-w-52 min-w-0 flex-1 items-center gap-1 rounded-md px-2 text-xs text-muted-foreground transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 sm:flex-none"
        disabled={
          configuration.currentModelConfig === null ||
          menuDisabled(configuration, commandPending)
        }
      >
        <span className="truncate">{triggerText}</span>
        <ChevronDownIcon className="size-3.5 shrink-0" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-64">
        <DropdownMenuGroup>
          <DropdownMenuLabel>{t("chat.composer.model")}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={selected?.model}
            onValueChange={(model) => {
              const descriptor = configuration.models.find(
                (candidate) => candidate.model === model
              )
              if (descriptor) configuration.selectModel(descriptor)
            }}
          >
            {configuration.models.map((descriptor) => (
              <DropdownMenuRadioItem
                key={descriptor.model}
                value={descriptor.model}
              >
                {formatModelLabel(descriptor.model)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuGroup>
        {configuration.existingAgent &&
          selected !== null &&
          selectedDescriptor !== undefined &&
          supportsThinkingControls(selectedDescriptor.parameters_schema) && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuSub>
                <DropdownMenuSubTrigger>
                  {t("chat.composer.thinking")}
                </DropdownMenuSubTrigger>
                <DropdownMenuSubContent>
                  <DropdownMenuRadioGroup
                    value={thinkingLevel(selected.parameters)}
                    onValueChange={(value) => {
                      if (
                        value === "disabled" ||
                        value === "high" ||
                        value === "max"
                      )
                        configuration.setThinkingLevel(value)
                    }}
                  >
                    <DropdownMenuRadioItem value="disabled">
                      {t("chat.composer.disabled")}
                    </DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value="high">
                      {t("chat.composer.high")}
                    </DropdownMenuRadioItem>
                    <DropdownMenuRadioItem value="max">
                      {t("chat.composer.max")}
                    </DropdownMenuRadioItem>
                  </DropdownMenuRadioGroup>
                </DropdownMenuSubContent>
              </DropdownMenuSub>
            </>
          )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function menuDisabled(
  configuration: ComposerConfiguration,
  commandPending: boolean
): boolean {
  return isModelConfigMenuDisabled({
    metadataLoading: configuration.metadataLoading,
    metadataError: configuration.metadataError !== null,
    turnRunning: configuration.turnRunning,
    existingAgent: configuration.existingAgent,
    currentModelConfig: configuration.currentModelConfig,
    commandPending,
  })
}

function thinkingLevel(
  parameters: Record<string, unknown>
): "disabled" | "high" | "max" {
  const thinking = parameters.thinking
  if (typeof thinking !== "object" || thinking === null) return "disabled"

  const level = thinking as Record<string, unknown>
  return level.reasoning_effort === "max"
    ? "max"
    : level.reasoning_effort === "high"
      ? "high"
      : "disabled"
}

function formatModelLabel(modelId: string): string {
  const displayName = modelDisplayName(modelId)
  return displayName.provider === null
    ? displayName.model
    : `${displayName.provider} · ${displayName.model}`
}
