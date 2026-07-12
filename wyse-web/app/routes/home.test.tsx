import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { describe, expect, it } from "vitest"

const source = readFileSync(
  fileURLToPath(new URL("./home.tsx", import.meta.url)),
  "utf8"
)

describe("Home horizontal sections", () => {
  it("uses a pinned horizontal track for the two primary sections", () => {
    expect(source).toContain('id: "home-horizontal"')
    expect(source).toContain('className="flex h-full w-[200vw]"')
    expect(source).toContain("pin: true")
    expect(source).toContain('ease: "none"')
  })
})
