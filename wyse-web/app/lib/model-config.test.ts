import { describe, expect, it } from "vitest"
import {
  nextDisplayedConfig,
  schemaDefault,
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
})
