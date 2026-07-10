"use client"

import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import {
  Children,
  createContext,
  useCallback,
  useContext,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type WheelEvent,
} from "react"

import {
  getPagerMotionPolicy,
  resolveSlideIndex,
  type PagerMotionPolicy,
} from "~/lib/workspace-pager"

const WORKSPACE_PAGER_DURATION_SECONDS = 0.65

gsap.registerPlugin(useGSAP)

type WorkspacePagerContextValue = {
  activeSlideIndex: number
  isTransitioning: boolean
  selectSlide: (slideIndex: number) => void
  slideCount: number
}

const WorkspacePagerContext = createContext<WorkspacePagerContextValue | null>(
  null
)

export type WorkspacePagerProps = {
  children: ReactNode
  initialSlideIndex?: number
}

export function useWorkspacePager(): WorkspacePagerContextValue {
  const context = useContext(WorkspacePagerContext)

  if (!context) {
    throw new Error("useWorkspacePager must be used within WorkspacePager")
  }

  return context
}

function getCurrentMotionPolicy(): PagerMotionPolicy {
  return getPagerMotionPolicy(
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  )
}

export function WorkspacePager({
  children,
  initialSlideIndex = 0,
}: WorkspacePagerProps) {
  const slides = useMemo(() => Children.toArray(children), [children])
  const slideCount = slides.length
  const rootRef = useRef<HTMLDivElement>(null)
  const trackRef = useRef<HTMLDivElement>(null)
  const tweenRef = useRef<gsap.core.Tween | null>(null)
  const transitionLockedRef = useRef(false)
  const [activeSlideIndex, setActiveSlideIndex] = useState(() =>
    resolveSlideIndex(0, initialSlideIndex, slideCount, false)
  )
  const [isTransitioning, setIsTransitioning] = useState(false)

  const { contextSafe } = useGSAP(
    () => {
      if (trackRef.current) {
        gsap.set(trackRef.current, { xPercent: -activeSlideIndex * 100 })
      }

      return () => {
        tweenRef.current?.kill()
      }
    },
    { scope: rootRef }
  )

  const selectSlide = useCallback(
    contextSafe((requestedIndex: number) => {
      const nextSlideIndex = resolveSlideIndex(
        activeSlideIndex,
        requestedIndex,
        slideCount,
        transitionLockedRef.current
      )

      if (nextSlideIndex === activeSlideIndex || !trackRef.current) {
        return
      }

      const motionPolicy = getCurrentMotionPolicy()
      setActiveSlideIndex(nextSlideIndex)

      if (motionPolicy === "instant") {
        gsap.set(trackRef.current, { xPercent: -nextSlideIndex * 100 })
        return
      }

      transitionLockedRef.current = true
      setIsTransitioning(true)
      tweenRef.current = gsap.to(trackRef.current, {
        duration: WORKSPACE_PAGER_DURATION_SECONDS,
        ease: "power2.inOut",
        xPercent: -nextSlideIndex * 100,
        onComplete: () => {
          transitionLockedRef.current = false
          setIsTransitioning(false)
          tweenRef.current = null
        },
      })
    }),
    [activeSlideIndex, contextSafe, slideCount]
  )

  const handleWheel = useCallback(
    (event: WheelEvent<HTMLDivElement>) => {
      if (event.deltaY === 0 || getCurrentMotionPolicy() === "instant") {
        return
      }

      event.preventDefault()
      selectSlide(activeSlideIndex + Math.sign(event.deltaY))
    },
    [activeSlideIndex, selectSlide]
  )

  const contextValue = useMemo(
    () => ({ activeSlideIndex, isTransitioning, selectSlide, slideCount }),
    [activeSlideIndex, isTransitioning, selectSlide, slideCount]
  )

  return (
    <WorkspacePagerContext.Provider value={contextValue}>
      <div
        className="workspace-pager-viewport"
        data-workspace-pager="viewport"
        onWheel={handleWheel}
        ref={rootRef}
      >
        <div
          className="workspace-pager-track"
          data-workspace-pager="track"
          ref={trackRef}
        >
          {slides.map((slide, index) => (
            <div
              aria-hidden={index !== activeSlideIndex}
              className="workspace-pager-slide"
              data-workspace-pager="slide"
              inert={index !== activeSlideIndex}
              key={index}
            >
              {slide}
            </div>
          ))}
        </div>
      </div>
    </WorkspacePagerContext.Provider>
  )
}
