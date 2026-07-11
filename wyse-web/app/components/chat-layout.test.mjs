import assert from "node:assert/strict"
import { existsSync } from "node:fs"
import { readFile } from "node:fs/promises"
import test from "node:test"

const homeUrl = new URL("../routes/home.tsx", import.meta.url)
const navbarUrl = new URL("./site-navbar.tsx", import.meta.url)
const workspaceUrl = new URL("./chat-workspace.tsx", import.meta.url)

test("the page keeps the overview hero and adds the Longzhong workspace", async () => {
  assert.equal(existsSync(workspaceUrl), true, "chat workspace is missing")

  const home = await readFile(homeUrl, "utf8")
  assert.match(home, /id="overview"/)
  assert.match(home, /<ChatWorkspace \/>/)
  assert.match(home, /href="#longzhong"/)
})

test("the glass navigation tracks both sections with a shared GSAP indicator", async () => {
  const navbar = await readFile(navbarUrl, "utf8")

  assert.match(navbar, /nav\.overview/)
  assert.match(navbar, /nav\.longzhong/)
  assert.match(navbar, /data-slot="section-indicator"/)
  assert.match(navbar, /scaleX/)
  assert.match(navbar, /scrollIntoView/)
  assert.match(navbar, /ScrollTrigger\.create\(\{[\s\S]*longzhong/)
  assert.match(
    navbar,
    /gsap\.to\(indicator, \{[\s\S]*x:[\s\S]*scaleX:[\s\S]*duration: reduceMotion \? 0 : 0\.5/
  )
})

test("the static workspace uses a side history rail and standard chat primitives", async () => {
  assert.equal(existsSync(workspaceUrl), true, "chat workspace is missing")

  const workspace = await readFile(workspaceUrl, "utf8")
  assert.match(workspace, /2xl:right-\[calc\(100%\+1\.5rem\)\]/)
  assert.match(workspace, /CardHeader/)
  assert.match(workspace, /MessageScrollerProvider/)
  assert.match(workspace, /MessageScrollerItem/)
  assert.match(workspace, /<Message /)
  assert.match(workspace, /<Bubble\b/)
  assert.match(workspace, /<Textarea/)
  assert.match(
    workspace,
    /<Card[\s\S]*?size="sm"[\s\S]*?className="[^"]*h-\[80dvh\][^"]*"/
  )
  assert.match(workspace, /CardContent className="[^"]*overflow-y-auto[^"]*"/)
  assert.doesNotMatch(workspace, /max-w-3xl/)
  assert.doesNotMatch(workspace, /event|事件/i)
})

test("the workspace keeps centered gutters around the chat stream", async () => {
  const workspace = await readFile(workspaceUrl, "utf8")
  const sectionClass = workspace.match(
    /id="longzhong"\s+className="([^"]+)"/
  )?.[1]

  assert.ok(sectionClass, "workspace section classes are missing")
  assert.match(sectionClass, /(?:^|\s)px-4(?:\s|$)/)
  assert.match(sectionClass, /(?:^|\s)md:px-8(?:\s|$)/)
  assert.match(workspace, /className="relative mx-auto w-full max-w-5xl"/)
  assert.match(workspace, /2xl:right-\[calc\(100%\+1\.5rem\)\]/)
})

test("the navbar and Longzhong chat canvas stay absolutely centered", async () => {
  const [navbar, workspace] = await Promise.all([
    readFile(navbarUrl, "utf8"),
    readFile(workspaceUrl, "utf8"),
  ])

  assert.match(workspace, /data-slot="chat-main"/)
  assert.match(navbar, /mx-auto flex h-12 w-full max-w-5xl/)
  assert.doesNotMatch(navbar, /navShellRef|gsap\.to\(navShell/)
  assert.match(workspace, /scroll-mt-20[^"]*pt-4/)
})
