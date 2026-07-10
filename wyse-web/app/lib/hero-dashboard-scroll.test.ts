import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"

import { shouldAutoScroll } from "./hero-dashboard-scroll"

test("allows automatic scroll only before user intent and without reduced motion", () => {
  assert.equal(shouldAutoScroll(false, false), true)
  assert.equal(shouldAutoScroll(true, false), false)
  assert.equal(shouldAutoScroll(false, true), false)
})

test("cancels and clears an active tween when the user signals intent", () => {
  const component = readFileSync(
    new URL("../components/hero-dashboard-scroll.tsx", import.meta.url),
    "utf8"
  )
  const handlerStart = component.indexOf("const cancelAutoScroll")
  const handlerEnd = component.indexOf("const timer", handlerStart)
  const cancellationCode = component.slice(handlerStart, handlerEnd)

  assert.match(cancellationCode, /tweenRef\.current\?\.kill\(\)/)
  assert.match(cancellationCode, /tweenRef\.current = null/)
})
