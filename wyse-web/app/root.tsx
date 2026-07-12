import { useState } from "react"
import { I18nextProvider, useTranslation } from "react-i18next"
import {
  Links,
  Meta,
  Outlet,
  Scripts,
  ScrollRestoration,
  isRouteErrorResponse,
  useRouteLoaderData,
} from "react-router"

import type { Route } from "./+types/root"
import { createI18n } from "./lib/i18n"
import { getRequestLanguage } from "./lib/locale"
import "./app.css"

const themeInitScript = `
(() => {
  const key = "wyse-theme";
  const stored = localStorage.getItem(key);
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const theme = stored === "light" || stored === "dark" ? stored : prefersDark ? "dark" : "light";
  document.documentElement.classList.toggle("light", theme === "light");
  document.documentElement.classList.toggle("dark", theme === "dark");
})();
`

export function loader({ request }: Route.LoaderArgs) {
  return { language: getRequestLanguage(request) }
}

export function Layout({ children }: { children: React.ReactNode }) {
  const language = useRouteLoaderData<typeof loader>("root")?.language ?? "en"
  const [i18n] = useState(() => createI18n(language))

  return (
    <html lang={language} suppressHydrationWarning>
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <title>Wyse Agent OS</title>
        <link rel="icon" type="image/svg+xml" href="/favicon.svg" />
        <script dangerouslySetInnerHTML={{ __html: themeInitScript }} />
        <Meta />
        <Links />
      </head>
      <body>
        <I18nextProvider i18n={i18n}>
          {children}
          <ScrollRestoration />
          <Scripts />
        </I18nextProvider>
      </body>
    </html>
  )
}

export default function App() {
  return <Outlet />
}

export function ErrorBoundary({ error }: Route.ErrorBoundaryProps) {
  const { t } = useTranslation()
  let message = t("errors.unexpectedTitle")
  let details = t("errors.unexpectedDetails")
  let stack: string | undefined

  if (isRouteErrorResponse(error)) {
    message = error.status === 404 ? "404" : t("errors.genericTitle")
    details =
      error.status === 404
        ? t("errors.notFoundDetails")
        : error.statusText || details
  } else if (import.meta.env.DEV && error && error instanceof Error) {
    details = error.message
    stack = error.stack
  }

  return (
    <main className="container mx-auto p-4 pt-16">
      <h1>{message}</h1>
      <p>{details}</p>
      {stack && (
        <pre className="w-full overflow-x-auto p-4">
          <code>{stack}</code>
        </pre>
      )}
    </main>
  )
}
