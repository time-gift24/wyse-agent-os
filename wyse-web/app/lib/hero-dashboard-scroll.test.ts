import assert from "node:assert/strict"
import test from "node:test"

import { shouldAutoScroll } from "./hero-dashboard-scroll"

test("allows automatic scroll only before user intent and without reduced motion", () => {
  assert.equal(shouldAutoScroll(false, false), true)
  assert.equal(shouldAutoScroll(true, false), false)
  assert.equal(shouldAutoScroll(false, true), false)
})
