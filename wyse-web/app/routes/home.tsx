import { ArrowRightIcon } from "lucide-react"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"

import { SiteNavbar } from "~/components/site-navbar"
import { StratumMark } from "~/components/stratum-mark"
import { Button, buttonVariants } from "~/components/ui/button"

export default function Home() {
  const { t } = useTranslation()

  return (
    <main className="min-h-[100dvh]">
      <SiteNavbar activeSection="overview" />

      <section className="flex min-h-[100dvh] w-full flex-col px-4 py-4 md:px-8 md:py-6">
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
              <Link
                to="/longzhong"
                viewTransition
                onClick={() => {
                  document.documentElement.dataset.navigationDirection =
                    "forward"
                }}
                className={buttonVariants({ size: "lg" })}
              >
                {t("actions.getStarted")}
                <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
              </Link>
              <Button variant="outline" size="lg">
                {t("actions.learnMore")}
              </Button>
            </div>
          </div>
        </div>
      </section>
    </main>
  )
}
