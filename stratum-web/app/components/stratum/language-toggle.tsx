import { useEffect } from "react"
import { LanguagesIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "~/components/ui/button"
import {
  LANGUAGE_STORAGE_KEY,
  serializeLanguageCookie,
  type Language,
} from "~/lib/locale"

function isLanguage(value: string | null): value is Language {
  return value === "en" || value === "zh"
}

type LanguageToggleProps = {
  compact?: boolean
}

export function LanguageToggle({ compact = false }: LanguageToggleProps) {
  const { i18n, t } = useTranslation()
  const language: Language = i18n.resolvedLanguage?.startsWith("zh")
    ? "zh"
    : "en"

  useEffect(() => {
    const savedLanguage = localStorage.getItem(LANGUAGE_STORAGE_KEY)
    if (isLanguage(savedLanguage) && savedLanguage !== language) {
      document.documentElement.lang = savedLanguage
      void i18n.changeLanguage(savedLanguage)
    }
  }, [i18n, language])

  const nextLanguage: Language = language === "en" ? "zh" : "en"
  const label =
    nextLanguage === "zh"
      ? t("language.switchToChinese")
      : t("language.switchToEnglish")

  return (
    <Button
      variant="ghost"
      size="lg"
      className={compact ? "size-11 p-0" : "min-h-11"}
      aria-label={label}
      title={label}
      onClick={() => {
        localStorage.setItem(LANGUAGE_STORAGE_KEY, nextLanguage)
        document.cookie = serializeLanguageCookie(nextLanguage)
        document.documentElement.lang = nextLanguage
        void i18n.changeLanguage(nextLanguage)
      }}
    >
      <LanguagesIcon data-icon="inline-start" aria-hidden="true" />
      {compact ? null : language === "en" ? "中" : "EN"}
    </Button>
  )
}
