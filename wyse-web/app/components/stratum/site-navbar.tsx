"use client"

import { useRef, type MouseEvent } from "react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { Link, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"

import GlassSurface from "~/components/react-bits/GlassSurface"
import { LanguageToggle } from "~/components/stratum/language-toggle"
import { StratumMark } from "~/components/stratum/stratum-mark"
import { ThemeToggle } from "~/components/stratum/theme-toggle"
import { Button } from "~/components/ui/button"
import {
  NavigationMenu,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
  navigationMenuTriggerStyle,
} from "~/components/ui/navigation-menu"
import { Separator } from "~/components/ui/separator"
import { cn } from "~/lib/utils"

gsap.registerPlugin(useGSAP)

type SiteSection = "overview" | "longzhong"

type SiteNavbarProps = {
  activeSection: SiteSection
}

export function SiteNavbar({ activeSection }: SiteNavbarProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const navRef = useRef<HTMLElement>(null)
  const shellRef = useRef<HTMLDivElement>(null)
  const glassRef = useRef<HTMLDivElement>(null)
  const sectionNavRef = useRef<HTMLDivElement>(null)
  const overviewLinkRef = useRef<HTMLAnchorElement>(null)
  const longzhongLinkRef = useRef<HTMLAnchorElement>(null)
  const indicatorRef = useRef<HTMLSpanElement>(null)
  const { contextSafe } = useGSAP({ scope: navRef })

  const isLongzhong = activeSection === "longzhong"

  const navigateWithTransition = contextSafe(
    (
      event: MouseEvent<HTMLAnchorElement>,
      to: string,
      direction: "forward" | "back"
    ) => {
      if (
        event.defaultPrevented ||
        event.button !== 0 ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.altKey
      )
        return

      event.preventDefault()
      if (
        (to === "/" && activeSection === "overview") ||
        (to === "/longzhong" && activeSection === "longzhong")
      )
        return

      document.documentElement.dataset.navigationDirection = direction
      const page = document.querySelector<HTMLElement>("[data-route-page]")
      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      if (!page || reduceMotion) {
        navigate(to)
        return
      }

      gsap.to(page, {
        xPercent: direction === "forward" ? -6 : 6,
        autoAlpha: 0,
        willChange: "transform, opacity",
        duration: 0.18,
        ease: "power2.in",
        overwrite: true,
        onComplete: () => navigate(to),
      })
    }
  )

  useGSAP(
    (_, contextSafe) => {
      const sectionNav = sectionNavRef.current
      const overviewLink = overviewLinkRef.current
      const longzhongLink = longzhongLinkRef.current
      const indicator = indicatorRef.current
      if (!sectionNav || !overviewLink || !longzhongLink || !indicator) return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const activeLink =
        activeSection === "overview" ? overviewLink : longzhongLink
      const navBounds = sectionNav.getBoundingClientRect()
      const linkBounds = activeLink.getBoundingClientRect()

      overviewLink.dataset.active = String(activeSection === "overview")
      longzhongLink.dataset.active = String(activeSection === "longzhong")
      overviewLink.toggleAttribute("aria-current", activeSection === "overview")
      longzhongLink.toggleAttribute(
        "aria-current",
        activeSection === "longzhong"
      )

      gsap.set(indicator, {
        x: linkBounds.left - navBounds.left,
        scaleX: 0,
        transformOrigin: "left center",
      })
      gsap.to(indicator, {
        scaleX: linkBounds.width,
        duration: reduceMotion ? 0 : 0.28,
        ease: "power2.out",
      })
    },
    { dependencies: [activeSection], scope: navRef, revertOnUpdate: true }
  )

  useGSAP(
    () => {
      const shell = shellRef.current
      if (!shell) return

      const glass = glassRef.current
      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches

      if (!isLongzhong) {
        gsap.set(shell, { "--navbar-max-width": "90rem" })
        if (glass) gsap.set(glass, { opacity: 0 })
        return
      }

      gsap.set(shell, { "--navbar-max-width": "90rem" })
      if (glass) gsap.set(glass, { opacity: 0 })

      const tl = gsap.timeline()
      tl.to(shell, {
        "--navbar-max-width": "64rem",
        duration: reduceMotion ? 0 : 0.55,
        ease: "power2.inOut",
      })
      tl.to(
        glass,
        {
          opacity: 1,
          duration: reduceMotion ? 0 : 0.35,
          ease: "power2.out",
        },
        "-=0.08"
      )
    },
    { dependencies: [isLongzhong], scope: navRef, revertOnUpdate: true }
  )

  const NavContent = (
    <div className="relative z-10 flex h-12 w-full items-center gap-4 px-3">
      <Link
        to="/"
        onClick={(event) => navigateWithTransition(event, "/", "back")}
        className="relative z-10 flex min-w-0 items-center gap-2 text-sm font-medium md:text-base"
        aria-label={`运筹 ${t("brand.home")}`}
      >
        <StratumMark animated={false} variant="compact" className="size-7" />
        <span className="truncate font-heading font-semibold">运筹</span>
      </Link>

      <div className="relative z-10 ml-auto flex items-center gap-3">
        <div ref={sectionNavRef} className="relative hidden md:block">
          <NavigationMenu className="flex-none">
            <NavigationMenuList>
              <NavigationMenuItem>
                <NavigationMenuLink
                  render={
                    <Link
                      ref={overviewLinkRef}
                      to="/"
                      onClick={(event) =>
                        navigateWithTransition(event, "/", "back")
                      }
                    />
                  }
                  className={cn(
                    navigationMenuTriggerStyle(),
                    "text-muted-foreground data-[active=true]:text-foreground"
                  )}
                  data-active={activeSection === "overview"}
                >
                  {t("nav.overview")}
                </NavigationMenuLink>
              </NavigationMenuItem>
              <NavigationMenuItem>
                <NavigationMenuLink
                  render={
                    <Link
                      ref={longzhongLinkRef}
                      to="/longzhong"
                      onClick={(event) =>
                        navigateWithTransition(event, "/longzhong", "forward")
                      }
                    />
                  }
                  className={cn(
                    navigationMenuTriggerStyle(),
                    "text-muted-foreground data-[active=true]:text-foreground"
                  )}
                  data-active={activeSection === "longzhong"}
                >
                  {t("nav.longzhong")}
                </NavigationMenuLink>
              </NavigationMenuItem>
            </NavigationMenuList>
          </NavigationMenu>
          <span
            ref={indicatorRef}
            data-slot="section-indicator"
            aria-hidden="true"
            className="absolute bottom-0 left-0 h-0.5 w-px origin-left bg-primary will-change-transform"
          />
        </div>
        <Separator orientation="vertical" className="hidden md:block" />
        <LanguageToggle />
        <ThemeToggle />
        <Button size="lg">{t("actions.signUp")}</Button>
      </div>
    </div>
  )

  return (
    <header
      ref={navRef}
      className="fixed inset-x-0 top-4 z-50 px-4 md:top-6 md:px-8"
    >
      <div
        ref={shellRef}
        className={cn(
          "navbar-shell relative isolate mx-auto flex items-stretch overflow-hidden rounded-full",
          isLongzhong && "navbar-shell--longzhong"
        )}
      >
        {isLongzhong && (
          <div
            ref={glassRef}
            className="absolute inset-0 opacity-0"
            aria-hidden="true"
          >
            <GlassSurface
              width="100%"
              height="100%"
              borderRadius={999}
              backgroundOpacity={0.12}
              backgroundOpacityDark={0.2}
              saturation={1.2}
              saturationDark={1.4}
              brightness={50}
              brightnessDark={40}
              opacity={0.93}
              opacityDark={0.85}
              blur={11}
              blurDark={14}
              className="navbar-glass"
            />
          </div>
        )}
        {NavContent}
      </div>
    </header>
  )
}
