import assert from "node:assert/strict"
import { access, readFile } from "node:fs/promises"
import test from "node:test"

const component = (name: string) =>
  readFile(new URL(`../components/${name}`, import.meta.url), "utf8")

test("composes exactly three fullscreen workspace slides", async () => {
  const home = await component("home-content.tsx")

  assert.match(home, /<WorkspacePager>/)
  assert.match(home, /data-workspace-slide="intro"/)
  assert.match(home, /<ChatWorkspace\s*\/>/)
  assert.match(home, /<OrchestrationWorkspace\s*\/>/)
  assert.equal((home.match(/data-workspace-slide=/g) ?? []).length, 1)
})

test("uses two right-aligned pager navigation entries", async () => {
  const navbar = await component("site-navbar.tsx")

  assert.match(navbar, /const WORKSPACE_NAVIGATION = \[/)
  assert.match(navbar, /label: "nav\.chat", slideIndex: 1/)
  assert.match(navbar, /label: "nav\.orchestration", slideIndex: 2/)
  assert.equal((navbar.match(/slideIndex:/g) ?? []).length, 2)
  assert.match(navbar, /selectSlide\(item\.slideIndex\)/)
})

test("uses shadcn's responsive Sidebar Sheet with a mobile trigger in both workspace panes", async () => {
  const [chat, orchestration, css] = await Promise.all([
    component("chat-workspace.tsx"),
    component("orchestration-workspace.tsx"),
    readFile(new URL("../app.css", import.meta.url), "utf8"),
  ])

  for (const source of [chat, orchestration]) {
    assert.match(source, /SidebarProvider/)
    assert.match(source, /SidebarTrigger/)
    assert.match(source, /<Sidebar\s+collapsible="offcanvas"/)
    assert.match(source, /<SidebarTrigger\s+className="[^"]*md:hidden"\s*\/>/)
    assert.match(source, /<SidebarInset/)

    const providerStart = source.indexOf("<SidebarProvider")
    const trigger = source.indexOf("<SidebarTrigger")
    const providerEnd = source.indexOf("</SidebarProvider>")

    assert.ok(providerStart < trigger && trigger < providerEnd)
  }

  assert.match(
    css,
    /\.wyse-workspace-inset\s*\{\s*@apply min-h-0 w-full bg-background;/
  )
})

test("places workspace navigation inside SidebarInset beside the fixed desktop Sidebar", async () => {
  const [chat, orchestration] = await Promise.all([
    component("chat-workspace.tsx"),
    component("orchestration-workspace.tsx"),
  ])

  for (const source of [chat, orchestration]) {
    assert.match(
      source,
      /<SidebarProvider[\s\S]*?<Sidebar\s+collapsible="offcanvas"[\s\S]*?<SidebarInset[^>]*>[\s\S]*?<SiteNavbar\s*\/>/
    )
  }
})

test("caps rendered Sidebar menu corners at six pixels", async () => {
  const [sidebar, css] = await Promise.all([
    component("ui/sidebar.tsx"),
    readFile(new URL("../app.css", import.meta.url), "utf8"),
  ])

  assert.match(sidebar, /rounded-\[calc\(var\(--radius-sm\)\+2px\)\]/)
  assert.match(css, /--radius-sm: 0\.25rem;/)
})

test("removes inactive pager slides from interaction and tab order", async () => {
  const pager = await component("workspace-pager.tsx")

  assert.match(
    pager,
    /aria-hidden=\{\s*index !== activeSlideIndex && index !== transitioningFromSlideIndex\s*\}/
  )
  assert.match(
    pager,
    /inert=\{\s*index !== activeSlideIndex && index !== transitioningFromSlideIndex\s*\}/
  )
})

test("hands focus to the incoming slide before making the outgoing slide inert", async () => {
  const pager = await component("workspace-pager.tsx")

  assert.match(
    pager,
    /const focusSlideIndexRef = useRef<number \| null>\(null\)/
  )
  assert.match(pager, /useLayoutEffect\(/)
  assert.match(pager, /slideRefs\.current\[focusSlideIndex\]\?\.focus\(\)/)
})

test("keeps SidebarInset workspace content out of nested main landmarks", async () => {
  const [chat, orchestration] = await Promise.all([
    component("chat-workspace.tsx"),
    component("orchestration-workspace.tsx"),
  ])

  for (const source of [chat, orchestration]) {
    assert.doesNotMatch(source, /<SidebarInset[^>]*>[\s\S]*?<main\b/)
    assert.match(
      source,
      /<section className="wyse-(?:chat|orchestration)-main"/
    )
  }
})

test("prevents the static chat composer from submitting the page", async () => {
  const chat = await component("chat-workspace.tsx")

  assert.match(
    chat,
    /<form\s+className="wyse-chat-composer"\s+onSubmit=\{\(event\) => event\.preventDefault\(\)\}/
  )
})

test("keeps workspace styling solid and removes the old dashboard", async () => {
  const css = await readFile(new URL("../app.css", import.meta.url), "utf8")

  assert.doesNotMatch(css, /wyse-dashboard|gradient|backdrop-filter|box-shadow/)
  await assert.rejects(
    access(new URL("../components/dashboard.tsx", import.meta.url))
  )
  await assert.rejects(
    access(new URL("./dashboard-sample.ts", import.meta.url))
  )
})
