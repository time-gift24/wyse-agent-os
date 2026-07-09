import { ArrowRightIcon, AtomIcon } from "lucide-react"

import { ThemeToggle } from "~/components/theme-toggle"
import { Badge } from "~/components/ui/badge"
import { Button } from "~/components/ui/button"
import {
  NavigationMenu,
  NavigationMenuItem,
  NavigationMenuLink,
  NavigationMenuList,
  navigationMenuTriggerStyle,
} from "~/components/ui/navigation-menu"
import { Separator } from "~/components/ui/separator"

export default function Home() {
  return (
    <main className="flex min-h-[100dvh]">
      <section className="flex min-h-[100dvh] w-full flex-col px-4 py-4 md:px-8 md:py-6">
        <header className="mx-auto flex w-full max-w-5xl items-center justify-between gap-4 px-3 py-2">
          <a
            href="/"
            className="flex min-w-0 items-center gap-2 text-sm font-medium md:text-base"
            aria-label="Wyse Agent OS home"
          >
            <AtomIcon className="size-5 shrink-0" aria-hidden="true" />
            <span className="truncate">Wyse Agent OS</span>
          </a>

          <NavigationMenu className="hidden flex-none md:flex">
            <NavigationMenuList>
              <NavigationMenuItem>
                <NavigationMenuLink
                  render={<a href="#runtime" />}
                  className={navigationMenuTriggerStyle()}
                >
                  Features
                </NavigationMenuLink>
              </NavigationMenuItem>
              <NavigationMenuItem>
                <NavigationMenuLink
                  render={<a href="#workflows" />}
                  className={navigationMenuTriggerStyle()}
                >
                  About
                </NavigationMenuLink>
              </NavigationMenuItem>
            </NavigationMenuList>
          </NavigationMenu>

          <div className="flex items-center gap-3">
            <Separator orientation="vertical" className="hidden md:block" />
            <ThemeToggle />
            <Button size="lg">Sign up</Button>
          </div>
        </header>

        <div className="flex flex-1 items-center justify-center py-16 md:py-24">
          <div className="flex max-w-4xl flex-col items-center gap-8 text-center">
            <Badge variant="outline" className="h-9 gap-2 px-2 text-sm">
              <Badge>New</Badge>
              <span>Runtime shell preview</span>
            </Badge>

            <div className="flex flex-col gap-5">
              <h1 className="font-heading text-5xl leading-[0.98] font-semibold tracking-tight text-balance md:text-7xl">
                Build typed agents
              </h1>
              <p className="mx-auto max-w-2xl text-base leading-relaxed text-muted-foreground md:text-lg">
                A Rust-first runtime for composing agents, tools, and reliable
                execution paths.
              </p>
            </div>

            <div className="flex flex-col items-center gap-3 sm:flex-row">
              <Button size="lg">
                Get started
                <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
              </Button>
              <Button variant="outline" size="lg">
                Learn more
              </Button>
            </div>
          </div>
        </div>
      </section>
    </main>
  )
}
