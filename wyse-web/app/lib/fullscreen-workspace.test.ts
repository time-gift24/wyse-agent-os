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

test("builds both workspace panes on the shadcn Sidebar foundation", async () => {
  const [chat, orchestration] = await Promise.all([
    component("chat-workspace.tsx"),
    component("orchestration-workspace.tsx"),
  ])

  for (const source of [chat, orchestration]) {
    assert.match(source, /SidebarProvider/)
    assert.match(source, /<Sidebar\s+collapsible="none"/)
    assert.match(source, /<SidebarInset/)
  }
})

test("removes inactive pager slides from interaction and tab order", async () => {
  const pager = await component("workspace-pager.tsx")

  assert.match(pager, /aria-hidden=\{index !== activeSlideIndex\}/)
  assert.match(pager, /inert=\{index !== activeSlideIndex\}/)
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
