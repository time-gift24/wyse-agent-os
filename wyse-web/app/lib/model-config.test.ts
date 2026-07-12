import { describe, expect, it } from "vitest"
import {
  configForModel,
  configForTemplate,
  nextDisplayedConfig,
  pendingConfigAfterAcceptance,
  schemaDefault,
  supportsThinkingControls,
  withThinkingLevel,
} from "./model-config"

describe("model configuration helpers", () => {
  it("clones the root schema default", () => {
    const defaultParameters = { thinking: { type: "disabled" } }
    const result = schemaDefault({ default: defaultParameters })

    expect(result).toEqual({
      thinking: { type: "disabled" },
    })
    expect(result).not.toBe(defaultParameters)
    expect(result.thinking).not.toBe(defaultParameters.thinking)
  })

  it("uses an empty object when the root default is not an object", () => {
    expect(schemaDefault({ default: ["unsupported"] })).toEqual({})
  })

  it("creates the configured DeepSeek thinking level", () => {
    expect(withThinkingLevel({}, "max")).toEqual({
      thinking: { type: "enabled", reasoning_effort: "max" },
    })
  })

  it("does not display a requested config until it is accepted", () => {
    const current = { model: "openai:test", parameters: {} }
    const requested = { model: "deepseek:test", parameters: {} }
    expect(nextDisplayedConfig(current, requested, false)).toEqual(current)
    expect(nextDisplayedConfig(current, requested, true)).toEqual(requested)
  })

  it("keeps a newer staged config after an earlier submission is accepted", () => {
    const submitted = { model: "openai:submitted", parameters: {} }
    const newer = { model: "openai:newer", parameters: {} }

    expect(pendingConfigAfterAcceptance(submitted, submitted)).toBeNull()
    expect(pendingConfigAfterAcceptance(newer, submitted)).toBe(newer)
  })

  it("displays the selected Agent template configuration before creation", () => {
    const template = {
      agent_name: "researcher",
      model_config: {
        model: "deepseek:deepseek-v4-pro",
        parameters: { thinking: { type: "enabled", reasoning_effort: "max" } },
      },
    }

    expect(configForTemplate(template)).toEqual(template.model_config)
  })

  it("initializes a switched model from its schema root default", () => {
    expect(
      configForModel({
        model: "deepseek:deepseek-v4-flash",
        parameters_schema: { default: { thinking: { type: "disabled" } } },
      })
    ).toEqual({
      model: "deepseek:deepseek-v4-flash",
      parameters: { thinking: { type: "disabled" } },
    })
  })

  it("recognizes the supported DeepSeek thinking schema", () => {
    expect(
      supportsThinkingControls({
        type: "object",
        properties: {
          thinking: {
            oneOf: [
              { properties: { type: { const: "disabled" } } },
              {
                properties: {
                  type: { const: "enabled" },
                  reasoning_effort: { enum: ["high", "max"] },
                },
              },
            ],
          },
        },
      })
    ).toBe(true)
    expect(supportsThinkingControls({ default: {} })).toBe(false)
    expect(
      supportsThinkingControls({
        properties: {
          thinking: {
            oneOf: [
              {
                properties: {
                  type: { const: "enabled" },
                  reasoning_effort: { enum: ["high", "max"] },
                },
              },
            ],
          },
        },
      })
    ).toBe(false)
  })
})
