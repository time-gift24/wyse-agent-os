import { OverviewWorkbench } from "~/components/stratum/overview-workbench"
import { RouteTransition } from "~/components/stratum/route-transition"

export default function Home() {
  return (
    <RouteTransition>
      <OverviewWorkbench />
    </RouteTransition>
  )
}
