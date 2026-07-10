export const LOCALE_STORAGE_KEY = "wyse-locale" as const

export type Locale = "zh" | "en"
export type MessageKey = keyof typeof messages.zh

const supportedLocales = new Set<Locale>(["zh", "en"])

export const messages = {
  zh: {
    "hero.title": "构建类型安全的智能体",
    "hero.body": "面向 Agent、工具与可靠执行路径的 Rust-first runtime。",
    "hero.enter": "进入工作台",
    "nav.product": "产品",
    "nav.workspace": "工作台",
    "nav.agents": "智能体",
    "nav.workflows": "工作流",
    "nav.runs": "运行记录",
    "nav.settings": "设置",
    "dashboard.eyebrow": "运行总览",
    "dashboard.title": "现在正在发生什么",
    "dashboard.body": "查看执行中的任务、排队工作与需要人工确认的运行。",
    "dashboard.primary": "查看全部运行",
    "status.running": "执行中",
    "status.queued": "已排队",
    "status.review": "待确认",
    "dashboard.recent": "近期执行",
    "dashboard.shortcuts": "快速打开",
    "dashboard.empty.title": "新的运行会显示在这里",
    "dashboard.empty.body": "连接运行时后，这里将展示实时执行记录。",
    "locale.toggle": "切换显示语言",
    "locale.option.zh": "中文",
    "locale.option.en": "EN",
  },
  en: {
    "hero.title": "Build typed agents",
    "hero.body": "A Rust-first runtime for agents, tools, and reliable execution paths.",
    "hero.enter": "Open workspace",
    "nav.product": "Product",
    "nav.workspace": "Workspace",
    "nav.agents": "Agents",
    "nav.workflows": "Workflows",
    "nav.runs": "Runs",
    "nav.settings": "Settings",
    "dashboard.eyebrow": "Runtime overview",
    "dashboard.title": "What is executing now",
    "dashboard.body": "Review active work, queued tasks, and runs that need confirmation.",
    "dashboard.primary": "View all runs",
    "status.running": "Running",
    "status.queued": "Queued",
    "status.review": "Needs review",
    "dashboard.recent": "Recent execution",
    "dashboard.shortcuts": "Quick open",
    "dashboard.empty.title": "New runs will appear here",
    "dashboard.empty.body": "Connect a runtime to show live execution records here.",
    "locale.toggle": "Change display language",
    "locale.option.zh": "中文",
    "locale.option.en": "EN",
  },
} as const satisfies Record<Locale, Record<string, string>>

export function isLocale(value: string | null): value is Locale {
  return value !== null && supportedLocales.has(value as Locale)
}

export function resolveLocale(
  stored: string | null,
  systemLanguage: string | undefined
): Locale {
  if (isLocale(stored)) return stored
  return systemLanguage?.toLowerCase().startsWith("en") ? "en" : "zh"
}
