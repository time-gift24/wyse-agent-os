import { createInstance } from "i18next"

import en from "../locales/en.json" with { type: "json" }
import zh from "../locales/zh.json" with { type: "json" }

import type { Language } from "./locale"

const resources = {
  en: { translation: en },
  zh: { translation: zh },
} as const

export function createI18n(language: Language) {
  const instance = createInstance()
  void instance.init({
    fallbackLng: "en",
    initAsync: false,
    interpolation: { escapeValue: false },
    lng: language,
    resources,
  })
  return instance
}
