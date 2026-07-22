import { ArrowRightIcon } from "lucide-react"
import { Link } from "react-router"
import { useTranslation } from "react-i18next"

import { RouteTransition } from "~/components/stratum/route-transition"
import { SiteNavbar } from "~/components/stratum/site-navbar"
import { StratumMark } from "~/components/stratum/stratum-mark"
import { Button, buttonVariants } from "~/components/ui/button"
import { cn } from "~/lib/utils"

export default function Home() {
  const { t } = useTranslation()

  return (
    <RouteTransition>
      <main className="min-h-[100dvh]">
        <SiteNavbar activeSection="overview" />

        <section className="flex min-h-[100dvh] w-full flex-col px-4 py-4 md:px-8 md:py-6">
          <div className="flex flex-1 items-center justify-center py-16 md:py-24">
            <div className="flex max-w-4xl flex-col items-center gap-10 text-center">
              <StratumMark className="size-32 md:size-40" />

              <div className="flex flex-col gap-5">
                <h1 className="type-hero text-balance text-foreground">
                  {t("hero.title")}
                </h1>
                <p className="type-body-large mx-auto max-w-2xl text-muted-foreground">
                  {t("hero.description")}
                </p>
              </div>

              <div className="flex flex-col items-center gap-3 sm:flex-row">
                <Link
                  to="/longzhong"
                  onClick={() => {
                    document.documentElement.dataset.navigationDirection =
                      "forward"
                  }}
                  className={cn(
                    buttonVariants({ size: "lg" }),
                    "h-11 rounded-md px-4 text-base font-normal shadow-[inset_0_0.5px_0_rgb(255_255_255/20%),inset_0_0_0_0.5px_rgb(0_0_0/20%),0_1px_2px_rgb(0_0_0/5%)]"
                  )}
                >
                  {t("actions.getStarted")}
                  <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
                </Link>
                <Button
                  variant="outline"
                  size="lg"
                  className="h-11 rounded-md border-stratum-line-strong px-4 text-base font-normal"
                >
                  {t("actions.learnMore")}
                </Button>
              </div>
            </div>
          </div>
        </section>
      </main>
    </RouteTransition>
  )
}
