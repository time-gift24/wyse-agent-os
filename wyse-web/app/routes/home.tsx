"use client"

import { useRef } from "react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { ScrollTrigger } from "gsap/ScrollTrigger"
import { ArrowRightIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { ChatWorkspace } from "~/components/chat-workspace"
import { SiteNavbar } from "~/components/site-navbar"
import { StratumMark } from "~/components/stratum-mark"
import { Button, buttonVariants } from "~/components/ui/button"

gsap.registerPlugin(useGSAP, ScrollTrigger)

export default function Home() {
  const { t } = useTranslation()
  const horizontalSectionRef = useRef<HTMLDivElement>(null)
  const horizontalTrackRef = useRef<HTMLDivElement>(null)

  useGSAP(
    () => {
      const section = horizontalSectionRef.current
      const track = horizontalTrackRef.current
      if (!section || !track) return

      const getDistance = () =>
        Math.max(0, track.scrollWidth - window.innerWidth)

      gsap.to(track, {
        x: () => -getDistance(),
        ease: "none",
        scrollTrigger: {
          id: "home-horizontal",
          trigger: section,
          start: "top top",
          end: () => `+=${getDistance()}`,
          pin: true,
          scrub: true,
          anticipatePin: 1,
          invalidateOnRefresh: true,
          onUpdate: (self) => {
            window.dispatchEvent(
              new CustomEvent("wyse:horizontal-section", {
                detail: self.progress < 0.5 ? "overview" : "longzhong",
              })
            )
          },
        },
      })
    },
    { scope: horizontalSectionRef }
  )

  return (
    <main className="min-h-[100dvh]">
      <div ref={horizontalSectionRef} className="h-[100dvh] overflow-hidden">
        <div ref={horizontalTrackRef} className="flex h-full w-[200vw]">
          <section
            id="overview"
            className="flex h-full w-screen shrink-0 flex-col px-4 py-4 md:px-8 md:py-6"
          >
            <SiteNavbar />

            <div className="flex flex-1 items-center justify-center py-16 md:py-24">
              <div className="flex max-w-4xl flex-col items-center gap-8 text-center">
                <StratumMark className="size-32 md:size-40" />

                <div className="flex flex-col gap-5">
                  <h1 className="font-heading text-5xl leading-[0.98] font-semibold tracking-tight text-balance md:text-7xl">
                    {t("hero.title")}
                  </h1>
                  <p className="mx-auto max-w-2xl text-base leading-relaxed text-muted-foreground md:text-lg">
                    {t("hero.description")}
                  </p>
                </div>

                <div className="flex flex-col items-center gap-3 sm:flex-row">
                  <a
                    href="#longzhong"
                    className={buttonVariants({ size: "lg" })}
                  >
                    {t("actions.getStarted")}
                    <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
                  </a>
                  <Button variant="outline" size="lg">
                    {t("actions.learnMore")}
                  </Button>
                </div>
              </div>
            </div>
          </section>
          <ChatWorkspace />
        </div>
      </div>
    </main>
  )
}
