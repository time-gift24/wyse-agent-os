import type { Locale } from "./locale"

export type RunStatus = "running" | "queued" | "review"

export type DashboardRun = {
  id: string
  title: string
  detail: string
  status: RunStatus
}

export type DashboardSample = {
  runs: DashboardRun[]
  shortcuts: { title: string; href: string }[]
}

export function getDashboardSample(locale: Locale): DashboardSample {
  if (locale === "en") {
    return {
      runs: [
        {
          id: "release-brief",
          title: "Release brief",
          detail: "Research agent",
          status: "running",
        },
        {
          id: "support-triage",
          title: "Support triage",
          detail: "Workflow",
          status: "queued",
        },
        {
          id: "policy-check",
          title: "Policy check",
          detail: "Review agent",
          status: "review",
        },
      ],
      shortcuts: [
        { title: "Agents", href: "#agents" },
        { title: "Workflows", href: "#workflows" },
      ],
    }
  }

  return {
    runs: [
      {
        id: "release-brief",
        title: "发布简报",
        detail: "调研 Agent",
        status: "running",
      },
      {
        id: "support-triage",
        title: "支持分流",
        detail: "工作流",
        status: "queued",
      },
      {
        id: "policy-check",
        title: "策略校验",
        detail: "审阅 Agent",
        status: "review",
      },
    ],
    shortcuts: [
      { title: "智能体", href: "#agents" },
      { title: "工作流", href: "#workflows" },
    ],
  }
}
