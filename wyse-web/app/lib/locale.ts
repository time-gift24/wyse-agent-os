export const LANGUAGE_COOKIE = "wyse-language"
export const LANGUAGE_STORAGE_KEY = LANGUAGE_COOKIE

export type Language = "en" | "zh"

function normalizeLanguage(value: string | null): Language {
  return value?.toLowerCase().startsWith("zh") ? "zh" : "en"
}

export function getRequestLanguage(request: Request): Language {
  const cookies = request.headers.get("cookie")?.split(";") ?? []
  const savedLanguage = cookies
    .map((cookie) => cookie.trim().split("=", 2))
    .find(([name]) => name === LANGUAGE_COOKIE)?.[1]

  return savedLanguage ? normalizeLanguage(savedLanguage) : "zh"
}

export function serializeLanguageCookie(language: Language) {
  return `${LANGUAGE_COOKIE}=${language}; Path=/; Max-Age=31536000; SameSite=Lax`
}
