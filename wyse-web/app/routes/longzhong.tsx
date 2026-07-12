import { ChatWorkspace } from "~/components/chat-workspace"
import { SiteNavbar } from "~/components/site-navbar"

export default function Longzhong() {
  return (
    <main>
      <SiteNavbar activeSection="longzhong" />
      <ChatWorkspace />
    </main>
  )
}
