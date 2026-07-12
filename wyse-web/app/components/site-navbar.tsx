"use client"

import { useRef, type MouseEvent } from "react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { ScrollToPlugin } from "gsap/ScrollToPlugin"
import { ScrollTrigger } from "gsap/ScrollTrigger"
import { useTranslation } from "react-i18next"

import GlassSurface from "~/components/GlassSurface"
import { LanguageToggle } from "~/components/language-toggle"
import { StratumMark } from "~/components/stratum-mark"
import { ThemeToggle } from "~/components/theme-toggle"
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

gsap.registerPlugin(useGSAP, ScrollTrigger, ScrollToPlugin)

const NAV_DURATION = 0.4

export function SiteNavbar() {
  const { t } = useTranslation()
  const navRef = useRef<HTMLElement>(null)
  const glassRef = useRef<HTMLDivElement>(null)
  const sectionNavRef = useRef<HTMLDivElement>(null)
  const overviewLinkRef = useRef<HTMLAnchorElement>(null)
  const longzhongLinkRef = useRef<HTMLAnchorElement>(null)
  const indicatorRef = useRef<HTMLSpanElement>(null)
  const setActiveSectionRef = useRef<
    ((section: "overview" | "longzhong") => void) | null
  >(null)

  const handleSectionNavigation = (event: MouseEvent<HTMLAnchorElement>) => {
    event.preventDefault()

    const section = event.currentTarget.hash.slice(1) as
      | "overview"
      | "longzhong"
    const target = document.getElementById(section)

    if (!target) {
      return
    }

    const reduceMotion = window.matchMedia(
      "(prefers-reduced-motion: reduce)"
    ).matches
    const horizontalTrigger = ScrollTrigger.getById("home-horizontal")
    const targetY = horizontalTrigger
      ? horizontalTrigger.start +
        (horizontalTrigger.end - horizontalTrigger.start) *
          (section === "overview" ? 0 : 1)
      : target.getBoundingClientRect().top + window.scrollY

    if (reduceMotion) {
      window.scrollTo(0, targetY)
    } else {
      gsap.to(window, {
        scrollTo: targetY,
        duration: NAV_DURATION,
        ease: "circ.inOut",
      })
    }

    window.history.replaceState(null, "", `#${section}`)
    setActiveSectionRef.current?.(section)
  }

  useGSAP(
    (_, contextSafe) => {
      const glass = glassRef.current

      const sectionNav = sectionNavRef.current
      const overviewLink = overviewLinkRef.current
      const longzhongLink = longzhongLinkRef.current
      const indicator = indicatorRef.current

      if (
        !glass ||
        !sectionNav ||
        !overviewLink ||
        !longzhongLink ||
        !indicator
      ) {
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

      let activeSection: "overview" | "longzhong" | null = null

      const setActiveSection = (
        section: "overview" | "longzhong",
        instant = false
      ) => {
        const target = section === "overview" ? overviewLink : longzhongLink
        const navBounds = sectionNav.getBoundingClientRect()
        const linkBounds = target.getBoundingClientRect()
        const wasSame = section === activeSection

        // Skip when same section and not forced instant — prevents
        // ScrollTrigger.onEnter from disrupting the click-driven animation
        if (wasSame && !instant) {
          return
        }

        activeSection = section
        overviewLink.dataset.active = String(section === "overview")
        longzhongLink.dataset.active = String(section === "longzhong")
        if (section === "overview") {
          overviewLink.setAttribute("aria-current", "page")
          longzhongLink.removeAttribute("aria-current")
        } else {
          overviewLink.removeAttribute("aria-current")
          longzhongLink.setAttribute("aria-current", "page")
        }

        if (instant) {
          gsap.set(indicator, {
            x: linkBounds.left - navBounds.left,
            scaleX: linkBounds.width,
          })
          return
        }

        gsap.to(indicator, {
          x: linkBounds.left - navBounds.left,
          scaleX: linkBounds.width,
          duration: reduceMotion ? 0 : NAV_DURATION,
          ease: "power3.in",
          overwrite: "auto",
        })
      }

      setActiveSectionRef.current = setActiveSection

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
      gsap.set(indicator, {
        scaleX: 0,
        transformOrigin: "left center",
      })
      setActiveSection("overview")

      ScrollTrigger.create({
        id: "site-navbar-glass",
        start: 0,
        end: "max",
        onRefresh: () => setGlassVisible(window.scrollY > 12),
        onUpdate: () => setGlassVisible(window.scrollY > 12),
      })

      const handleHorizontalSectionChange = contextSafe((event: Event) => {
        const section = (event as CustomEvent<"overview" | "longzhong">).detail
        if (section === "overview" || section === "longzhong") {
          setActiveSection(section)
        }
      })
      window.addEventListener(
        "wyse:horizontal-section",
        handleHorizontalSectionChange
      )

      return () => {
        window.removeEventListener(
          "wyse:horizontal-section",
          handleHorizontalSectionChange
        )
        setActiveSectionRef.current = null
      }
    },
    { scope: navRef }
  )

  return (
    <header
      ref={navRef}
      className="fixed inset-x-0 top-4 z-50 px-4 md:top-6 md:px-8"
    >
      <div className="wyse-content-width relative isolate mx-auto flex h-12 items-center gap-4 px-3">
        <div ref={glassRef} aria-hidden="true" className="site-navbar-glass">
          <GlassSurface
            width="100%"
            height="100%"
            borderRadius={12}
            borderWidth={0.1}
            brightness={68}
            opacity={0.5}
            blur={100}
            displace={2.2}
            backgroundOpacity={0.05}
            saturation={1.15}
            distortionScale={-40}
            redOffset={0}
            greenOffset={2}
            blueOffset={4}
            mixBlendMode="normal"
          />
        </div>

        <a
          href="/"
          className="relative z-10 flex min-w-0 items-center gap-2 text-sm font-medium md:text-base"
          aria-label={`运筹 ${t("brand.home")}`}
        >
          <StratumMark animated={false} variant="compact" className="size-7" />
          <span className="truncate font-heading font-semibold">运筹</span>
        </a>

        <div className="relative z-10 ml-auto flex items-center gap-3">
          <div ref={sectionNavRef} className="relative hidden md:block">
            <NavigationMenu className="flex-none">
              <NavigationMenuList>
                <NavigationMenuItem>
                  <NavigationMenuLink
                    render={
                      <a
                        ref={overviewLinkRef}
                        href="#overview"
                        onClick={handleSectionNavigation}
                      />
                    }
                    className={cn(
                      navigationMenuTriggerStyle(),
                      "text-muted-foreground data-[active=true]:text-foreground"
                    )}
                    data-active="true"
                    aria-current="page"
                  >
                    {t("nav.overview")}
                  </NavigationMenuLink>
                </NavigationMenuItem>
                <NavigationMenuItem>
                  <NavigationMenuLink
                    render={
                      <a
                        ref={longzhongLinkRef}
                        href="#longzhong"
                        onClick={handleSectionNavigation}
                      />
                    }
                    className={cn(
                      navigationMenuTriggerStyle(),
                      "text-muted-foreground data-[active=true]:text-foreground"
                    )}
                    data-active="false"
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
    </header>
  )
}
