"use client"

import { useEffect, useRef } from "react"
import gsap from "gsap"
import { ScrollToPlugin } from "gsap/ScrollToPlugin"

import { shouldAutoScroll } from "../lib/hero-dashboard-scroll"

gsap.registerPlugin(ScrollToPlugin)

export function HeroDashboardScroll() {
  const tweenRef = useRef<gsap.core.Tween | null>(null)

  useEffect(() => {
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)")
    let hasUserIntent = false

    const cancelAutoScroll = () => {
      hasUserIntent = true
      window.clearTimeout(timer)
    }

    const timer = window.setTimeout(() => {
      if (!shouldAutoScroll(hasUserIntent, reducedMotion.matches)) {
        return
      }

      const target = document.getElementById("dashboard")
      if (!target) {
        return
      }

      tweenRef.current = gsap.to(window, {
        duration: 0.9,
        ease: "power2.inOut",
        scrollTo: { y: target, offsetY: 96 },
      })
    }, 1800)

    const userIntentEvents = ["pointerdown", "wheel", "touchstart", "keydown"] as const
    userIntentEvents.forEach((eventName) => {
      window.addEventListener(eventName, cancelAutoScroll, { once: true })
    })

    return () => {
      window.clearTimeout(timer)
      userIntentEvents.forEach((eventName) => {
        window.removeEventListener(eventName, cancelAutoScroll)
      })
      tweenRef.current?.kill()
    }
  }, [])

  return null
}
