import { ChatWorkspace } from "~/components/stratum/chat-workspace"
import { RouteTransition } from "~/components/stratum/route-transition"
import { SiteNavbar } from "~/components/stratum/site-navbar"

export default function Longzhong() {
  return (
    <RouteTransition>
      <main>
        <SiteNavbar activeSection="longzhong" />
        <ChatWorkspace />
      </main>
    </RouteTransition>
  )
}
