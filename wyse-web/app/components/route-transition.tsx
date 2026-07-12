"use client"

import { useRef, type ReactNode } from "react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"

gsap.registerPlugin(useGSAP)

type RouteTransitionProps = {
  children: ReactNode
}

export function RouteTransition({ children }: RouteTransitionProps) {
  const pageRef = useRef<HTMLDivElement>(null)

  useGSAP(
    () => {
      const page = pageRef.current
      if (!page) return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const direction = document.documentElement.dataset.navigationDirection

      gsap.fromTo(
        page,
        {
          xPercent: direction === "back" ? -6 : 6,
          autoAlpha: 0,
          willChange: "transform, opacity",
        },
        {
          xPercent: 0,
          autoAlpha: 1,
          duration: reduceMotion ? 0 : 0.32,
          ease: "power3.out",
          clearProps: "willChange",
        }
      )
    },
    { scope: pageRef }
  )

  return (
    <div ref={pageRef} data-route-page>
      {children}
    </div>
  )
}
