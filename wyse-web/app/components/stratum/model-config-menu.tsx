import { ChevronDownIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
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
  supportsThinkingControls,
} from "~/lib/model-config"

type ModelConfigMenuProps = {
  configuration: ComposerConfiguration
  commandPending: boolean
}

type ModelConfigMenuContentProps = Pick<ModelConfigMenuProps, "configuration">

export function ModelConfigMenu({
  configuration,
  commandPending,
}: ModelConfigMenuProps) {
  const { t } = useTranslation()
  const current = configuration.currentModelConfig
  const disabled = isModelConfigMenuDisabled({
    metadataLoading: configuration.metadataLoading,
    metadataError: configuration.metadataError !== null,
    turnRunning: configuration.turnRunning,
    existingAgent: configuration.existingAgent,
    currentModelConfig: current,
    commandPending,
  })
  const triggerText = configuration.metadataLoading
    ? t("chat.composer.loadingConfiguration")
    : configuration.agentName === null || current === null
      ? t("chat.composer.selectAgent")
      : `${configuration.agentName} · ${current.model}`

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        aria-label={triggerText}
        className="inline-flex h-8 max-w-56 items-center gap-1 rounded-md px-2 text-xs text-muted-foreground transition-colors outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50"
        disabled={disabled}
      >
        <span className="truncate">{triggerText}</span>
        <ChevronDownIcon className="size-3.5 shrink-0" aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        {configuration.existingAgent ? (
          <ExistingAgentMenu configuration={configuration} />
        ) : (
          <NewAgentMenu configuration={configuration} />
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function NewAgentMenu({ configuration }: ModelConfigMenuContentProps) {
  const { t } = useTranslation()
  const current = configuration.currentModelConfig

  return (
    <>
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>
          {t("chat.composer.agent")}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
          <DropdownMenuRadioGroup
            value={configuration.selectedTemplate?.agent_name}
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
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      {current !== null && (
        <>
          <DropdownMenuSeparator />
          <DropdownMenuLabel>{t("chat.composer.model")}</DropdownMenuLabel>
          <DropdownMenuItem disabled>{current.model}</DropdownMenuItem>
          <DropdownMenuItem disabled>
            {JSON.stringify(current.parameters)}
          </DropdownMenuItem>
        </>
      )}
    </>
  )
}

function ExistingAgentMenu({ configuration }: ModelConfigMenuContentProps) {
  const { t } = useTranslation()
  const selected = configuration.selectedModelConfig
  const selectedDescriptor = configuration.models.find(
    (descriptor) => descriptor.model === selected?.model
  )

  return (
    <>
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>
          {t("chat.composer.model")}
        </DropdownMenuSubTrigger>
        <DropdownMenuSubContent>
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
                {descriptor.model}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      {selected !== null &&
        selectedDescriptor !== undefined &&
        supportsThinkingControls(selectedDescriptor.parameters_schema) && (
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
        )}
    </>
  )
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
