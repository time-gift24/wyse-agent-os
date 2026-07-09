import { useEffect, useState } from "react"
import { MoonIcon, SunIcon } from "lucide-react"

import { Switch } from "~/components/ui/switch"

const STORAGE_KEY = "wyse-theme"

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
  localStorage.setItem(STORAGE_KEY, theme)
}

export function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>("light")

  useEffect(() => {
    setTheme(getInitialTheme())
  }, [])

  const isDark = theme === "dark"

  return (
    <div className="flex items-center gap-2">
      {isDark ? (
        <MoonIcon className="size-4 text-muted-foreground" aria-hidden="true" />
      ) : (
        <SunIcon className="size-4 text-muted-foreground" aria-hidden="true" />
      )}
      <Switch
        checked={isDark}
        aria-label="Toggle dark theme"
        onCheckedChange={(checked) => {
          const nextTheme = checked ? "dark" : "light"
          applyTheme(nextTheme)
          setTheme(nextTheme)
        }}
      />
    </div>
  )
}
