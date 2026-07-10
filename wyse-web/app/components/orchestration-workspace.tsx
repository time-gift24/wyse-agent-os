"use client"

import { BotIcon, CableIcon, WorkflowIcon } from "lucide-react"

import { LocaleToggle } from "~/components/locale-toggle"
import { useLocale } from "~/components/locale-provider"
import { SiteNavbar } from "~/components/site-navbar"
import { ThemeToggle } from "~/components/theme-toggle"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from "~/components/ui/sidebar"

const orchestrationItems = [
  { icon: BotIcon, label: "orchestration.agents" },
  { icon: WorkflowIcon, label: "orchestration.workflows" },
  { icon: CableIcon, label: "orchestration.tools" },
] as const

export function OrchestrationWorkspace() {
  const { t } = useLocale()

  return (
    <section
      data-workspace-slide="orchestration"
      className="wyse-workspace-slide"
    >
      <SiteNavbar />
      <div className="wyse-workspace-body">
        <SidebarProvider className="wyse-workspace-shell">
          <Sidebar collapsible="none" className="wyse-workspace-sidebar">
            <SidebarHeader className="wyse-workspace-sidebar-header">
              <span className="wyse-workspace-sidebar-title">
                {t("orchestration.library")}
              </span>
            </SidebarHeader>
            <SidebarContent>
              <SidebarGroup>
                <SidebarGroupLabel>
                  {t("orchestration.library")}
                </SidebarGroupLabel>
                <SidebarMenu>
                  {orchestrationItems.map((item) => {
                    const Icon = item.icon

                    return (
                      <SidebarMenuItem key={item.label}>
                        <SidebarMenuButton
                          isActive={item.label === "orchestration.workflows"}
                        >
                          <Icon aria-hidden="true" />
                          <span>{t(item.label)}</span>
                        </SidebarMenuButton>
                      </SidebarMenuItem>
                    )
                  })}
                </SidebarMenu>
              </SidebarGroup>
            </SidebarContent>
            <SidebarFooter className="wyse-workspace-sidebar-footer">
              <LocaleToggle />
              <ThemeToggle />
            </SidebarFooter>
          </Sidebar>
          <SidebarInset className="wyse-workspace-inset">
            <section className="wyse-orchestration-main">
              <header className="wyse-orchestration-header">
                <p className="wyse-chat-eyebrow">{t("nav.orchestration")}</p>
                <h2 className="wyse-orchestration-title">
                  {t("orchestration.title")}
                </h2>
              </header>
              <div className="wyse-orchestration-flow">
                <div className="wyse-orchestration-node">
                  {t("orchestration.trigger")}
                </div>
                <div className="wyse-orchestration-node">
                  {t("orchestration.agent")}
                </div>
                <div className="wyse-orchestration-node wyse-orchestration-node--active">
                  {t("orchestration.result")}
                </div>
              </div>
            </section>
          </SidebarInset>
        </SidebarProvider>
      </div>
    </section>
  )
}
