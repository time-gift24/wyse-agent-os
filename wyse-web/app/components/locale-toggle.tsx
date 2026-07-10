"use client"

import { useLocale } from "~/components/locale-provider"
import { Button } from "~/components/ui/button"

export function LocaleToggle() {
  const { locale, setLocale, t } = useLocale()

  return (
    <div aria-label={t("locale.toggle")} className="flex gap-1" role="group">
      <Button
        aria-pressed={locale === "zh"}
        onClick={() => setLocale("zh")}
        size="sm"
        type="button"
        variant={locale === "zh" ? "secondary" : "ghost"}
      >
        {t("locale.option.zh")}
      </Button>
      <Button
        aria-pressed={locale === "en"}
        onClick={() => setLocale("en")}
        size="sm"
        type="button"
        variant={locale === "en" ? "secondary" : "ghost"}
      >
        {t("locale.option.en")}
      </Button>
    </div>
  )
}
