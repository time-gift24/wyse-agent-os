"use client"

import { useRef } from "react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"

import compactStratumMarkSvg from "~/assets/stratum-mark-compact.svg?raw"
import stratumMarkSvg from "~/assets/stratum-mark.svg?raw"
import { cn } from "~/lib/utils"

import type { ComponentProps } from "react"

type StratumMarkProps = Omit<
  ComponentProps<"span">,
  "children" | "dangerouslySetInnerHTML"
> & {
  animated?: boolean
  variant?: "default" | "compact"
}

gsap.registerPlugin(useGSAP)

export function StratumMark({
  animated = true,
  className,
  variant = "default",
  ...props
}: StratumMarkProps) {
  const markRef = useRef<HTMLSpanElement>(null)
  const svgMarkup =
    variant === "compact" ? compactStratumMarkSvg : stratumMarkSvg

  useGSAP(
    () => {
      const svg = markRef.current?.querySelector<SVGSVGElement>("svg")
      const diamond = markRef.current?.querySelector<SVGGElement>(
        "[data-stratum-diamond]"
      )

      if (!animated || !svg || !diamond) {
        return
      }

      const viewBox = svg.viewBox.baseVal
      const centerX = viewBox.x + viewBox.width / 2
      const centerY = viewBox.y + viewBox.height / 2
      const media = gsap.matchMedia()

      media.add("(prefers-reduced-motion: no-preference)", () => {
        const motion = {
          rotation: -16,
          scale: 0.78,
          opacity: 0,
        }
        const renderDiamond = () => {
          diamond.setAttribute(
            "transform",
            `translate(${centerX} ${centerY}) rotate(${motion.rotation}) scale(${motion.scale}) translate(${-centerX} ${-centerY})`
          )
          diamond.style.opacity = String(motion.opacity)
        }
        const timeline = gsap.timeline({ onUpdate: renderDiamond })

        renderDiamond()

        timeline
          .to(motion, {
            duration: 0.9,
            ease: "back.out(1.35)",
            opacity: 1,
            rotation: 0,
            scale: 1,
          })
          .to(motion, {
            duration: 14,
            ease: "none",
            repeat: -1,
            rotation: 90,
          })

        return () => {
          timeline.kill()
          diamond.removeAttribute("transform")
          diamond.style.removeProperty("opacity")
        }
      })

      return () => media.revert()
    },
    {
      dependencies: [animated, variant],
      revertOnUpdate: true,
      scope: markRef,
    }
  )

  return (
    <span
      ref={markRef}
      aria-hidden="true"
      {...props}
      className={cn(
        "inline-block shrink-0 leading-none [&>svg]:block [&>svg]:size-full [&>svg]:overflow-visible",
        variant === "compact" && "stratum-mark--compact",
        className
      )}
      dangerouslySetInnerHTML={{ __html: svgMarkup }}
    />
  )
}
