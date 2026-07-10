import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const componentUrl = new URL("./stratum-mark.tsx", import.meta.url)
const assetUrl = new URL("../assets/stratum-mark.svg", import.meta.url)
const compactAssetUrl = new URL(
  "../assets/stratum-mark-compact.svg",
  import.meta.url
)
const appCssUrl = new URL("../app.css", import.meta.url)

test("the supplied Stratum SVG is background-free and keeps its original geometry", async () => {
  const asset = await readFile(assetUrl, "utf8")

  assert.match(asset, /viewBox="480 108 712 712"/)
  assert.match(asset, /data-stratum-weave/)
  assert.match(asset, /data-stratum-diamond/)
  assert.match(asset, /M 624\.64,488/)
  assert.match(asset, /M 873\.32,404\.71/)
  assert.match(asset, /#65aa9f/)
  assert.doesNotMatch(asset, /<rect\b/)
})

test("the compact Stratum SVG remains a transparent white mark", async () => {
  const asset = await readFile(compactAssetUrl, "utf8")
  const fills = [...asset.matchAll(/fill="([^"]+)"/g)].map(([, fill]) => fill)

  assert.ok(fills.length >= 9)
  assert.deepEqual(new Set(fills), new Set(["#fff"]))
  assert.match(asset, /stroke-width="12"/)
  assert.match(asset, /stroke-linecap="round"/)
  assert.match(asset, /stroke-linejoin="round"/)
  assert.doesNotMatch(asset, /<rect\b/)
})

test("the Stratum component scopes motion to its diamond", async () => {
  const [component, appCss] = await Promise.all([
    readFile(componentUrl, "utf8"),
    readFile(appCssUrl, "utf8"),
  ])

  assert.match(component, /stratum-mark\.svg\?raw/)
  assert.match(component, /stratum-mark-compact\.svg\?raw/)
  assert.match(component, /variant = "default"/)
  assert.match(component, /variant === "compact"/)
  assert.match(component, /dangerouslySetInnerHTML/)
  assert.match(component, /data-stratum-diamond/)
  assert.match(component, /querySelector<SVGGElement>/)
  assert.match(component, /viewBox\.baseVal/)
  assert.match(component, /prefers-reduced-motion: no-preference/)
  assert.match(component, /gsap\.timeline/)
  assert.match(component, /setAttribute\(\s*"transform"/)
  assert.doesNotMatch(component, /svgOrigin/)
  assert.doesNotMatch(component, /transformBox/)
  assert.doesNotMatch(component, /gsap\.(?:to|from|fromTo)\([^)]*weave/s)
  assert.doesNotMatch(appCss, /drop-shadow/)
})
