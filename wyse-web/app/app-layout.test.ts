import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { describe, expect, it } from "vitest"

const source = readFileSync(
  fileURLToPath(new URL("./app.css", import.meta.url)),
  "utf8"
)

describe("responsive chat layout tokens", () => {
  it("keeps the medium desktop canvas and history rail compact", () => {
    expect(source).toContain("clamp(40rem, 58vw, 64rem)")
    expect(source).toContain("clamp(13rem, 16vw, 18rem)")
    expect(source).toContain("@media (min-width: 1200px)")
  })
})
