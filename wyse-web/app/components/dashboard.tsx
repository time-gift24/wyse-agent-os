import { getDashboardSample, type RunStatus } from "~/lib/dashboard-sample"
import type { Locale, MessageKey } from "~/lib/locale"

const statusKeys: Record<RunStatus, MessageKey> = {
  running: "status.running",
  queued: "status.queued",
  review: "status.review",
}

const statuses: RunStatus[] = ["running", "queued", "review"]

type DashboardProps = {
  locale: Locale
  t: (key: MessageKey) => string
}

export function Dashboard({ locale, t }: DashboardProps) {
  const { runs, shortcuts } = getDashboardSample(locale)

  return (
    <section id="dashboard" className="wyse-dashboard">
      <header className="wyse-dashboard-overview">
        <div className="wyse-dashboard-copy">
          <p className="wyse-dashboard-eyebrow">{t("dashboard.eyebrow")}</p>
          <h1 className="wyse-dashboard-title">{t("dashboard.title")}</h1>
          <p className="wyse-dashboard-body">{t("dashboard.body")}</p>
        </div>
        <a className="wyse-dashboard-primary" href="#runs">
          {t("dashboard.primary")}
        </a>
      </header>

      <div className="wyse-dashboard-grid">
        {statuses.map((status) => (
          <div key={status} className="wyse-dashboard-card">
            <span className={`wyse-status wyse-status--${status}`}>
              {t(statusKeys[status])}
            </span>
          </div>
        ))}
      </div>

      <section id="runs" className="wyse-dashboard-timeline">
        <h2 className="wyse-dashboard-section-title">
          {t("dashboard.recent")}
        </h2>
        {runs.map((run) => (
          <article key={run.id} className="wyse-dashboard-run">
            <div className="wyse-dashboard-run-copy">
              <h3 className="wyse-dashboard-run-title">{run.title}</h3>
              <p className="wyse-dashboard-run-detail">{run.detail}</p>
            </div>
            <span className={`wyse-status wyse-status--${run.status}`}>
              {t(statusKeys[run.status])}
            </span>
          </article>
        ))}
      </section>

      <section
        className="wyse-dashboard-shortcuts"
        aria-labelledby="shortcuts-title"
      >
        <h2 id="shortcuts-title" className="wyse-dashboard-section-title">
          {t("dashboard.shortcuts")}
        </h2>
        <nav
          className="wyse-dashboard-shortcut-list"
          aria-label={t("dashboard.shortcuts")}
        >
          {shortcuts.map((shortcut) => (
            <a
              key={shortcut.href}
              className="wyse-dashboard-shortcut"
              href={shortcut.href}
            >
              {shortcut.title}
            </a>
          ))}
        </nav>
        <div className="wyse-dashboard-empty">
          <h3 className="wyse-dashboard-empty-title">
            {t("dashboard.empty.title")}
          </h3>
          <p>{t("dashboard.empty.body")}</p>
        </div>
      </section>
    </section>
  )
}
