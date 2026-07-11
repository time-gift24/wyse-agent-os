import { describe, expect, it } from "vitest"

import { highlightCode } from "~/components/ai-elements/code-block"

describe("CodeBlock", () => {
  it("uses the official Shiki renderer for tool JSON", async () => {
    const [light, dark] = await highlightCode('{"path":"README.md"}', "json")

    expect(light).toContain("<pre")
    expect(dark).toContain("<pre")
  })
})
