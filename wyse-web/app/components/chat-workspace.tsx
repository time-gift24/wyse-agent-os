"use client"

import { SendIcon } from "lucide-react"

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
  SidebarTrigger,
} from "~/components/ui/sidebar"

export function ChatWorkspace() {
  const { t } = useLocale()

  return (
    <section data-workspace-slide="chat" className="wyse-workspace-slide">
      <SidebarProvider className="wyse-workspace-shell">
        <Sidebar collapsible="offcanvas" className="wyse-workspace-sidebar">
          <SidebarHeader className="wyse-workspace-sidebar-header">
            <span className="wyse-workspace-sidebar-title">
              {t("chat.sessions")}
            </span>
          </SidebarHeader>
          <SidebarContent>
            <SidebarGroup>
              <SidebarGroupLabel>{t("chat.recent")}</SidebarGroupLabel>
              <SidebarMenu>
                <SidebarMenuItem>
                  <SidebarMenuButton isActive>
                    <span>{t("chat.thread")}</span>
                  </SidebarMenuButton>
                </SidebarMenuItem>
              </SidebarMenu>
            </SidebarGroup>
          </SidebarContent>
          <SidebarFooter className="wyse-workspace-sidebar-footer">
            <LocaleToggle />
            <ThemeToggle />
          </SidebarFooter>
        </Sidebar>
        <SidebarInset className="wyse-workspace-inset">
          <SiteNavbar />
          <div className="wyse-workspace-body">
            <SidebarTrigger className="absolute top-16 left-3 md:hidden" />
            <section className="wyse-chat-main">
              <div className="wyse-chat-copy">
                <p className="wyse-chat-eyebrow">{t("nav.chat")}</p>
                <h2 className="wyse-chat-title">{t("chat.title")}</h2>
                <p className="wyse-chat-body">{t("chat.body")}</p>
              </div>
              <form
                className="wyse-chat-composer"
                onSubmit={(event) => event.preventDefault()}
              >
                <label className="wyse-visually-hidden" htmlFor="chat-prompt">
                  {t("chat.prompt")}
                </label>
                <input
                  className="wyse-chat-input"
                  id="chat-prompt"
                  placeholder={t("chat.prompt")}
                />
                <button className="wyse-chat-send" type="submit">
                  <span>{t("chat.send")}</span>
                  <SendIcon aria-hidden="true" />
                </button>
              </form>
            </section>
          </div>
        </SidebarInset>
      </SidebarProvider>
    </section>
  )
}
