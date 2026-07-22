import { useEffect, useState } from "react"
import { MoonIcon, SunIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "~/components/ui/button"

const STORAGE_KEY = "stratum-theme"
const THEME_CHANGE_EVENT = "stratum-theme-change"

type Theme = "light" | "dark"

function getInitialTheme(): Theme {
  if (typeof document === "undefined") {
    return "light"
  }

  return document.documentElement.classList.contains("dark") ? "dark" : "light"
}

function applyTheme(theme: Theme) {
  document.documentElement.classList.toggle("light", theme === "light")
  document.documentElement.classList.toggle("dark", theme === "dark")
  try {
    localStorage.setItem(STORAGE_KEY, theme)
  } catch {
    // 存储不可用时，当前文档仍然可以正常切换主题。
  }
  window.dispatchEvent(new Event(THEME_CHANGE_EVENT))
}

export function ThemeToggle() {
  const { t } = useTranslation()
  const [theme, setTheme] = useState<Theme>("light")

  useEffect(() => {
    const syncTheme = () => setTheme(getInitialTheme())
    syncTheme()
    window.addEventListener(THEME_CHANGE_EVENT, syncTheme)
    window.addEventListener("storage", syncTheme)
    return () => {
      window.removeEventListener(THEME_CHANGE_EVENT, syncTheme)
      window.removeEventListener("storage", syncTheme)
    }
  }, [])

  const isDark = theme === "dark"
  const label = isDark ? t("theme.useLight") : t("theme.useDark")

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon"
      className="size-11 rounded-lg"
      aria-label={label}
      title={label}
      onClick={() => {
        const nextTheme = isDark ? "light" : "dark"
        applyTheme(nextTheme)
        setTheme(nextTheme)
      }}
    >
      {isDark ? (
        <SunIcon className="size-[18px] stroke-[1.8]" aria-hidden="true" />
      ) : (
        <MoonIcon className="size-[18px] stroke-[1.8]" aria-hidden="true" />
      )}
    </Button>
  )
}
