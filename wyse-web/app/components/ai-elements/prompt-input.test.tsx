import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it } from "vitest"

import {
  PromptInput,
  PromptInputBody,
  PromptInputFooter,
  PromptInputSubmit,
  PromptInputTextarea,
  PromptInputTools,
} from "~/components/ai-elements/prompt-input"

describe("PromptInput", () => {
  it("composes the official prompt body, tools, and submit primitives", () => {
    const html = renderToStaticMarkup(
      <PromptInput onSubmit={() => {}}>
        <PromptInputBody>
          <PromptInputTextarea aria-label="Message" defaultValue="" />
        </PromptInputBody>
        <PromptInputFooter>
          <PromptInputTools>Connected</PromptInputTools>
          <PromptInputSubmit ariaLabel="发送" disabled />
        </PromptInputFooter>
      </PromptInput>
    )

    expect(html).toContain('data-slot="prompt-input"')
    expect(html).toContain('data-slot="prompt-input-body"')
    expect(html).toContain('data-slot="prompt-input-tools"')
    expect(html).toContain('type="submit"')
    expect(html).toContain('aria-label="发送"')
    expect(html).not.toMatch(/(?:^|\s)border-t(?:\s|")/)
  })
})
