"use client"

import { useRef, type MouseEvent, type ReactNode } from "react"
import { UserPlusIcon } from "lucide-react"
import { useGSAP } from "@gsap/react"
import gsap from "gsap"
import { Link, useNavigate } from "react-router"
import { useTranslation } from "react-i18next"

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
  const sectionNavRef = useRef<HTMLDivElement>(null)
  const leftSlotRef = useRef<HTMLDivElement>(null)
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
        duration: reduceMotion ? 0 : 0.2,
        ease: "power2.out",
      })
    },
    { dependencies: [activeSection], scope: navRef, revertOnUpdate: true }
  )

  const NavContent = (
    <div className="flex h-14 w-full items-center gap-2 px-2 sm:gap-4 sm:px-3">
      {leftSlot ? (
        <div className="flex items-center 2xl:hidden">{leftSlot}</div>
      ) : null}
      <Link
        to="/"
        onClick={(event) => navigateWithTransition(event, "/", "back")}
        className="flex shrink-0 items-center gap-1.5 text-sm font-normal md:text-base"
        aria-label={`运筹 ${t("brand.home")}`}
      >
        <StratumMark animated={false} variant="compact" className="size-7" />
        <span className="hidden font-heading font-semibold sm:inline">
          运筹
        </span>
      </Link>

      <div className="relative z-10 ml-auto flex min-w-0 items-center gap-1 sm:gap-3">
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
                    "h-11 rounded-md px-3 text-sm font-normal text-muted-foreground data-[active=true]:text-foreground"
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
                    "h-11 rounded-md px-3 text-sm font-normal text-muted-foreground data-[active=true]:text-foreground"
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
        {rightSlot}
        <div className="sm:hidden">
          <LanguageToggle compact />
        </div>
        <div className="hidden sm:block">
          <LanguageToggle />
        </div>
        <div className="shrink-0">
          <ThemeToggle />
        </div>
        <Button
          size="icon-lg"
          className="size-11 rounded-md sm:hidden"
          aria-label={t("actions.signUp")}
          title={t("actions.signUp")}
        >
          <UserPlusIcon aria-hidden="true" />
        </Button>
        <Button
          size="lg"
          className="hidden h-11 rounded-md px-4 text-base font-normal shadow-[inset_0_0.5px_0_rgb(255_255_255/20%),inset_0_0_0_0.5px_rgb(0_0_0/20%),0_1px_2px_rgb(0_0_0/5%)] sm:inline-flex"
        >
          {t("actions.signUp")}
        </Button>
      </div>
    </div>
  )

  return (
    <header ref={navRef} className="fixed inset-x-0 top-0 z-50 mt-3 md:mt-4">
      <div
        className={cn(
          "navbar-shell mx-auto flex items-stretch overflow-hidden rounded-xl border border-stratum-line bg-stratum-paper",
          isLongzhong && "navbar-shell--longzhong"
        )}
      >
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
