export type ModelConfig = {
  model: string
  parameters: Record<string, unknown>
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

export function nextDisplayedConfig(
  current: ModelConfig,
  requested: ModelConfig,
  accepted: boolean
): ModelConfig {
  return accepted ? requested : current
}
