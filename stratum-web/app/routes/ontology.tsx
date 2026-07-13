import { OntologyWorkspace } from "~/components/stratum/ontology-workspace"
import { RouteTransition } from "~/components/stratum/route-transition"
import { SiteNavbar } from "~/components/stratum/site-navbar"

export default function Ontology() {
  return (
    <RouteTransition>
      <main className="min-h-[100dvh]">
        <SiteNavbar activeSection="ontology" />
        <OntologyWorkspace />
      </main>
    </RouteTransition>
  )
}
