"use client"

import { ArrowRightIcon } from "lucide-react"

import { Dashboard } from "~/components/dashboard"
import { HeroDashboardScroll } from "~/components/hero-dashboard-scroll"
import { useLocale } from "~/components/locale-provider"
import { SiteNavbar } from "~/components/site-navbar"
import { StratumMark } from "~/components/stratum-mark"
import { Button } from "~/components/ui/button"

export function HomeContent() {
  const { locale, t } = useLocale()

  return (
    <>
      <HeroDashboardScroll />
      <SiteNavbar />
      <main className="wyse-home">
        <section className="wyse-home-hero">
          <div className="wyse-home-hero__content">
            <StratumMark className="wyse-home-hero__mark" />
            <div className="wyse-home-hero__copy">
              <h1 className="wyse-home-hero__title">{t("hero.title")}</h1>
              <p className="wyse-home-hero__body">{t("hero.body")}</p>
            </div>
            <Button size="lg" render={<a href="#dashboard" />}>
              {t("hero.enter")}
              <ArrowRightIcon data-icon="inline-end" aria-hidden="true" />
            </Button>
          </div>
        </section>
        <Dashboard locale={locale} t={t} />
      </main>
    </>
  )
}
