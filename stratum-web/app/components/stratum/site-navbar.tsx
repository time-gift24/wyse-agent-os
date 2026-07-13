"use client"

import {
  useEffect,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react"
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

const isDarkMode = () => {
  if (typeof document === "undefined") return false
  return document.documentElement.classList.contains("dark")
}

type SiteSection = "overview" | "longzhong" | "ontology"

type SiteNavbarProps = {
  activeSection: SiteSection
  leftSlot?: ReactNode
  rightSlot?: ReactNode
}

export function SiteNavbar({
  activeSection,
  leftSlot,
  rightSlot,
}: SiteNavbarProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const navRef = useRef<HTMLElement>(null)
  const shellRef = useRef<HTMLDivElement>(null)
  const glassRef = useRef<HTMLDivElement>(null)
  const sectionNavRef = useRef<HTMLDivElement>(null)
  const leftSlotRef = useRef<HTMLDivElement>(null)
  const overviewLinkRef = useRef<HTMLAnchorElement>(null)
  const longzhongLinkRef = useRef<HTMLAnchorElement>(null)
  const ontologyLinkRef = useRef<HTMLAnchorElement>(null)
  const indicatorRef = useRef<HTMLSpanElement>(null)
  const { contextSafe } = useGSAP({ scope: navRef })

  const isLongzhong = activeSection === "longzhong"
  const [isDark, setIsDark] = useState(() => isDarkMode())

  useEffect(() => {
    if (typeof window === "undefined") return

    const update = () => setIsDark(isDarkMode())
    const media = window.matchMedia("(prefers-color-scheme: dark)")
    const observer = new MutationObserver(update)

    media.addEventListener("change", update)
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    })

    return () => {
      media.removeEventListener("change", update)
      observer.disconnect()
    }
  }, [])

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
      const activePath =
        activeSection === "overview"
          ? "/"
          : activeSection === "longzhong"
            ? "/longzhong"
            : "/ontology"
      if (to === activePath) return

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
        duration: 0.22,
        ease: "sine.in",
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
      const ontologyLink = ontologyLinkRef.current
      const indicator = indicatorRef.current
      if (
        !sectionNav ||
        !overviewLink ||
        !longzhongLink ||
        !ontologyLink ||
        !indicator
      )
        return

      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches
      const links = [
        ["overview", overviewLink],
        ["longzhong", longzhongLink],
        ["ontology", ontologyLink],
      ] as const
      const activeLink = links.find(
        ([section]) => section === activeSection
      )?.[1]
      if (!activeLink) return
      const navBounds = sectionNav.getBoundingClientRect()
      const linkBounds = activeLink.getBoundingClientRect()

      for (const [section, link] of links) {
        const active = section === activeSection
        link.dataset.active = String(active)
        link.toggleAttribute("aria-current", active)
      }

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
      const leftSlotEl = leftSlotRef.current
      const reduceMotion = window.matchMedia(
        "(prefers-reduced-motion: reduce)"
      ).matches

      if (!isLongzhong) {
        gsap.set(shell, { "--navbar-max-width": "90rem" })
        if (glass) gsap.set(glass, { opacity: 0 })
        if (leftSlotEl) gsap.set(leftSlotEl, { autoAlpha: 0, x: -8 })
        return
      }

      if (glass) gsap.set(glass, { opacity: 0 })
      if (leftSlotEl) gsap.set(leftSlotEl, { autoAlpha: 0, x: -8 })

      const tl = gsap.timeline()
      tl.to(
        glass,
        {
          opacity: 1,
          duration: reduceMotion ? 0 : 0.4,
          ease: "sine.inOut",
        },
        "-=0.45"
      )
      if (leftSlotEl) {
        tl.to(
          leftSlotEl,
          {
            autoAlpha: 1,
            x: 0,
            duration: reduceMotion ? 0 : 0.3,
            ease: "sine.out",
          },
          "-=0.3"
        )
      }
    },
    { dependencies: [isLongzhong], scope: navRef, revertOnUpdate: true }
  )

  const NavContent = (
    <div className="relative z-10 flex h-12 w-full items-center gap-4 px-3">
      {leftSlot ? (
        <div className="flex items-center 2xl:hidden">{leftSlot}</div>
      ) : null}
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
                        navigateWithTransition(
                          event,
                          "/longzhong",
                          activeSection === "ontology" ? "back" : "forward"
                        )
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
              <NavigationMenuItem>
                <NavigationMenuLink
                  render={
                    <Link
                      ref={ontologyLinkRef}
                      to="/ontology"
                      onClick={(event) =>
                        navigateWithTransition(event, "/ontology", "forward")
                      }
                    />
                  }
                  className={cn(
                    navigationMenuTriggerStyle(),
                    "text-muted-foreground data-[active=true]:text-foreground"
                  )}
                  data-active={activeSection === "ontology"}
                >
                  {t("nav.modeling")}
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
        {rightSlot}
        <LanguageToggle />
        <ThemeToggle />
        <Button size="lg">{t("actions.signUp")}</Button>
      </div>
    </div>
  )

  return (
    <header
      ref={navRef}
      className={cn(
        "fixed inset-x-0 top-0 z-50 mt-4 md:mt-6",
        isLongzhong ? "px-0" : "px-4 md:px-8"
      )}
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
              borderRadius={2}
              backgroundOpacity={isDark ? 0.35 : 0.1}
              saturation={isDark ? 1.6 : 1.1}
              brightness={isDark ? 28 : 78}
              opacity={isDark ? 0.78 : 0.72}
              blur={isDark ? 16 : 14}
              displace={4}
              className="navbar-glass"
            />
          </div>
        )}
        {NavContent}
      </div>

      {leftSlot ? (
        <div
          ref={leftSlotRef}
          data-slot="navbar-left-slot"
          className={cn(
            "pointer-events-none absolute top-1/2 hidden -translate-y-1/2 2xl:block",
            "right-[calc(50%+var(--content-half-width)+1rem)]"
          )}
        >
          <div className="pointer-events-auto">{leftSlot}</div>
        </div>
      ) : null}
    </header>
  )
}
