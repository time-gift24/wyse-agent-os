export type ModelConfig = {
  model: string
  parameters: Record<string, unknown>
}

export type ModelDisplayName = {
  provider: string | null
  model: string
}

export type ModelDescriptor = {
  model: string
  parameters_schema: unknown
}

export type AgentTemplateView = {
  agent_name: string
  model_config: ModelConfig
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value)

export function schemaDefault(schema: unknown): Record<string, unknown> {
  if (!isRecord(schema) || !isRecord(schema.default)) return {}

  return structuredClone(schema.default)
}

export function configForTemplate(template: AgentTemplateView): ModelConfig {
  return structuredClone(template.model_config)
}

export function configForModel(descriptor: ModelDescriptor): ModelConfig {
  return {
    model: descriptor.model,
    parameters: schemaDefault(descriptor.parameters_schema),
  }
}

export function modelDisplayName(modelId: string): ModelDisplayName {
  const separator = modelId.indexOf(":")
  if (separator <= 0 || separator === modelId.length - 1)
    return { provider: null, model: modelId }

  const provider = modelId.slice(0, separator)
  return {
    provider: provider.charAt(0).toUpperCase() + provider.slice(1),
    model: modelId.slice(separator + 1),
  }
}

export function supportsThinkingControls(schema: unknown): boolean {
  if (!isRecord(schema) || !isRecord(schema.properties)) return false

  const thinking = schema.properties.thinking
  if (!isRecord(thinking) || !Array.isArray(thinking.oneOf)) return false

  let hasDisabled = false
  let hasEnabledWithLevels = false
  for (const option of thinking.oneOf) {
    if (!isRecord(option) || !isRecord(option.properties)) continue

    const type = option.properties.type
    if (!isRecord(type)) continue
    if (type.const === "disabled") hasDisabled = true
    if (type.const !== "enabled") continue

    const reasoningEffort = option.properties.reasoning_effort
    hasEnabledWithLevels =
      isRecord(reasoningEffort) &&
      Array.isArray(reasoningEffort.enum) &&
      reasoningEffort.enum.includes("high") &&
      reasoningEffort.enum.includes("max")
  }

  return hasDisabled && hasEnabledWithLevels
}

export function withThinkingLevel(
  parameters: Record<string, unknown>,
  level: "disabled" | "high" | "max"
): Record<string, unknown> {
  return {
    ...parameters,
    thinking:
      level === "disabled"
        ? { type: "disabled" }
        : { type: "enabled", reasoning_effort: level },
  }
}
