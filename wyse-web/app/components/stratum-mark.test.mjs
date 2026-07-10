import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const componentUrl = new URL("./stratum-mark.tsx", import.meta.url)
const navbarUrl = new URL("./site-navbar.tsx", import.meta.url)
const homeUrl = new URL("../routes/home.tsx", import.meta.url)
const homeContentUrl = new URL("./home-content.tsx", import.meta.url)
const assetUrl = new URL("../assets/stratum-mark.svg", import.meta.url)
const compactAssetUrl = new URL(
  "../assets/stratum-mark-compact.svg",
  import.meta.url
)
const appCssUrl = new URL("../app.css", import.meta.url)
const dashboardUrl = new URL("./dashboard.tsx", import.meta.url)
const dashboardSampleUrl = new URL(
  "../lib/dashboard-sample.ts",
  import.meta.url
)

test("the supplied Stratum SVG is background-free and keeps its original geometry", async () => {
  const asset = await readFile(assetUrl, "utf8").catch((error) => {
    if (error.code === "ENOENT") {
      return ""
    }

    throw error
  })

  assert.match(asset, /viewBox="480 108 712 712"/)
  assert.match(asset, /data-stratum-weave/)
  assert.match(asset, /data-stratum-diamond/)
  assert.match(asset, /M 624\.64,488/)
  assert.match(asset, /M 873\.32,404\.71/)
  assert.match(asset, /#65aa9f/)
  assert.doesNotMatch(asset, /<rect\b/)
  assert.doesNotMatch(asset, /#faf5f4/i)
})

test("the compact Stratum SVG uses the muted ink token at small sizes", async () => {
  const [asset, appCss] = await Promise.all([
    readFile(compactAssetUrl, "utf8").catch((error) => {
      if (error.code === "ENOENT") {
        return ""
      }

      throw error
    }),
    readFile(appCssUrl, "utf8"),
  ])
  const fills = [...asset.matchAll(/fill="([^"]+)"/g)].map(([, fill]) => fill)
  const strokes = [...asset.matchAll(/stroke="([^"]+)"/g)].map(
    ([, stroke]) => stroke
  )

  assert.ok(fills.length >= 9)
  assert.deepEqual(new Set(fills), new Set(["#fff"]))
  assert.deepEqual(new Set(strokes), new Set(["#fff"]))
  assert.match(asset, /stroke-width="12"/)
  assert.match(asset, /stroke-linecap="round"/)
  assert.match(asset, /stroke-linejoin="round"/)
  assert.doesNotMatch(asset, /<rect\b/)
  assert.match(appCss, /--stratum-mark-compact: var\(--wyse-ink-muted\)/)
  assert.match(
    appCss,
    /\.stratum-mark--compact path\s*{[^}]*fill: var\(--stratum-mark-compact\);[^}]*stroke: var\(--stratum-mark-compact\)/s
  )
})

test("the Stratum mark inlines the SVG and animates only its diamond", async () => {
  const [component, navbar, homeContent, appCss] = await Promise.all([
    readFile(componentUrl, "utf8"),
    readFile(navbarUrl, "utf8"),
    readFile(homeContentUrl, "utf8"),
    readFile(appCssUrl, "utf8"),
  ])

  assert.match(component, /stratum-mark\.svg\?raw/)
  assert.match(component, /stratum-mark-compact\.svg\?raw/)
  assert.match(component, /variant = "default"/)
  assert.match(component, /variant === "compact"/)
  assert.match(component, /stratum-mark--compact/)
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

  assert.match(
    navbar,
    /<StratumMark animated=\{false\} variant="compact" className="size-7" \/>/
  )
  assert.match(navbar, />运筹<\/span>/)
  assert.match(navbar, />Stratum<\/span>/)
  assert.match(homeContent, /<StratumMark className="wyse-home-hero__mark" \/>/)
  assert.match(appCss, /\.stratum-mark--compact/)
  assert.doesNotMatch(appCss, /drop-shadow/)
})

test("the dashboard remains a static run-first sample", async () => {
  const [dashboard, sample] = await Promise.all([
    readFile(dashboardUrl, "utf8"),
    readFile(dashboardSampleUrl, "utf8"),
  ])

  assert.match(dashboard, /id="dashboard"/)
  assert.match(dashboard, /getDashboardSample/)
  assert.match(dashboard, /status\.running/)
  assert.match(dashboard, /status\.queued/)
  assert.match(dashboard, /status\.review/)
  assert.doesNotMatch(sample, /fetch\(|axios|\b\d+(?:\.\d+)?(?:%|ms)\b/)
})

test("the navbar groups localized dashboard navigation", async () => {
  const navbar = await readFile(navbarUrl, "utf8")

  assert.match(navbar, /NavigationMenuContent/)
  assert.match(navbar, /NavigationMenuTrigger/)
  assert.match(navbar, /t\("nav\.product"\)/)
  assert.match(navbar, /href="#dashboard"/)
  assert.match(navbar, /href="#agents"/)
  assert.match(navbar, /href="#workflows"/)
  assert.match(navbar, /href="#runs"/)
  assert.match(navbar, /<LocaleToggle\s*\/>/)
})

test("the home composes localized hero and dashboard content", async () => {
  const [home, content] = await Promise.all([
    readFile(homeUrl, "utf8"),
    readFile(homeContentUrl, "utf8"),
  ])

  assert.match(home, /<LocaleProvider>/)
  assert.match(home, /<HomeContent\s*\/>/)
  assert.match(content, /<HeroDashboardScroll\s*\/>/)
  assert.match(content, /<SiteNavbar\s*\/>/)
  assert.match(content, /<StratumMark/)
  assert.match(content, /<Dashboard/)
})

test("dashboard component rules keep visual declarations in Tailwind apply utilities", async () => {
  const appCss = await readFile(appCssUrl, "utf8")
  const dashboardRules = [
    ...appCss.matchAll(/\.wyse-dashboard[^\{]*\{([^}]*)\}/g),
  ]
    .map(([, declarations]) => declarations)
    .join("\n")

  assert.doesNotMatch(
    dashboardRules,
    /(?:^|\n)\s*(?:border|background|box-shadow|backdrop-filter|-webkit-backdrop-filter):/
  )
})

test("dashboard quick-open links have reachable semantic targets", async () => {
  const dashboard = await readFile(dashboardUrl, "utf8")

  assert.equal((dashboard.match(/id="agents"/g) ?? []).length, 1)
  assert.equal((dashboard.match(/id="workflows"/g) ?? []).length, 1)
})
