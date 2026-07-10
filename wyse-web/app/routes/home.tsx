import { ArrowRightIcon } from "lucide-react"

import { SiteNavbar } from "~/components/site-navbar"
import { Badge } from "~/components/ui/badge"
import { Button } from "~/components/ui/button"

export default function Home() {
  return (
    <main className="flex min-h-[100dvh]">
      <section className="flex min-h-[100dvh] w-full flex-col px-4 py-4 md:px-8 md:py-6">
        <SiteNavbar />

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
