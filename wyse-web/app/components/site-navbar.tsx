"use client"

import { useRef } from "react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { ScrollTrigger } from "gsap/ScrollTrigger"

import GlassSurface from "~/components/GlassSurface"
import { useLocale } from "~/components/locale-provider"
import { LocaleToggle } from "~/components/locale-toggle"
import { StratumMark } from "~/components/stratum-mark"
import { ThemeToggle } from "~/components/theme-toggle"
import { Button } from "~/components/ui/button"
import {
  NavigationMenu,
  NavigationMenuContent,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
  NavigationMenuTrigger,
} from "~/components/ui/navigation-menu"
import { Separator } from "~/components/ui/separator"

gsap.registerPlugin(useGSAP, ScrollTrigger)

export function SiteNavbar() {
  const navRef = useRef<HTMLElement>(null)
  const glassRef = useRef<HTMLDivElement>(null)
  const { t } = useLocale()

  useGSAP(
    () => {
      const glass = glassRef.current

      if (!glass) {
        return
      }

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const showGlass = () => {
        gsap.to(glass, {
          autoAlpha: 1,
          scale: 1,
          duration: reduceMotion ? 0 : 0.28,
          ease: "power2.out",
          overwrite: true,
        })
      }
      const hideGlass = () => {
        gsap.to(glass, {
          autoAlpha: 0,
          scale: 0.985,
          duration: reduceMotion ? 0 : 0.2,
          ease: "power2.out",
          overwrite: true,
        })
      }

      let glassVisible = window.scrollY > 12

      const setGlassVisible = (nextVisible: boolean) => {
        if (nextVisible === glassVisible) {
          return
        }

        glassVisible = nextVisible
        if (nextVisible) {
          showGlass()
        } else {
          hideGlass()
        }
      }

      gsap.set(glass, {
        autoAlpha: glassVisible ? 1 : 0,
        scale: glassVisible ? 1 : 0.985,
        transformOrigin: "50% 50%",
      })

      ScrollTrigger.create({
        id: "site-navbar-glass",
        start: 0,
        end: "max",
        onRefresh: () => setGlassVisible(window.scrollY > 12),
        onUpdate: () => setGlassVisible(window.scrollY > 12),
      })
    },
    { scope: navRef }
  )

  return (
    <header
      ref={navRef}
      className="fixed inset-x-0 top-4 z-50 px-4 md:top-6 md:px-8"
    >
      <div className="site-navbar-shell">
        <div ref={glassRef} aria-hidden="true" className="site-navbar-glass">
          <GlassSurface
            width="100%"
            height="100%"
            borderRadius={999}
            borderWidth={0.085}
            brightness={58}
            opacity={0.86}
            blur={12}
            displace={0.35}
            backgroundOpacity={0.18}
            saturation={1.35}
            distortionScale={-120}
            redOffset={0}
            greenOffset={10}
            blueOffset={22}
            mixBlendMode="screen"
          />
        </div>

        <a
          href="/"
          className="site-navbar-brand"
          aria-label="运筹 Stratum home"
        >
          <StratumMark animated={false} variant="compact" className="size-7" />
          <span className="site-navbar-brand-copy">
            <span className="font-heading font-semibold">运筹</span>
            <span className="text-xs text-muted-foreground">Stratum</span>
          </span>
        </a>

        <NavigationMenu className="relative z-10 hidden flex-none md:flex">
          <NavigationMenuList>
            <NavigationMenuItem>
              <NavigationMenuTrigger>{t("nav.product")}</NavigationMenuTrigger>
              <NavigationMenuContent>
                <NavigationMenuLink render={<a href="#dashboard" />}>
                  <dl>
                    <dt>{t("nav.workspace")}</dt>
                    <dd>{t("nav.workspace.description")}</dd>
                  </dl>
                </NavigationMenuLink>
                <NavigationMenuLink render={<a href="#agents" />}>
                  <dl>
                    <dt>{t("nav.agents")}</dt>
                    <dd>{t("nav.agents.description")}</dd>
                  </dl>
                </NavigationMenuLink>
                <NavigationMenuLink render={<a href="#workflows" />}>
                  <dl>
                    <dt>{t("nav.workflows")}</dt>
                    <dd>{t("nav.workflows.description")}</dd>
                  </dl>
                </NavigationMenuLink>
                <NavigationMenuLink render={<a href="#runs" />}>
                  <dl>
                    <dt>{t("nav.runs")}</dt>
                    <dd>{t("nav.runs.description")}</dd>
                  </dl>
                </NavigationMenuLink>
              </NavigationMenuContent>
            </NavigationMenuItem>
          </NavigationMenuList>
        </NavigationMenu>

        <div className="site-navbar-actions">
          <Separator orientation="vertical" className="hidden md:block" />
          <LocaleToggle />
          <ThemeToggle />
          <Button
            className="site-navbar-cta max-sm:hidden"
            render={<a href="#dashboard" />}
            size="lg"
          >
            {t("hero.enter")}
          </Button>
        </div>
      </div>
    </header>
  )
}
