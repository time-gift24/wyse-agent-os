import { ChatWorkspace } from "~/components/chat-workspace"
import { RouteTransition } from "~/components/route-transition"
import { SiteNavbar } from "~/components/site-navbar"

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
