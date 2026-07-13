"use client"

import { useEffect, useRef, useState } from "react"
import {
  InfoIcon,
  ListTreeIcon,
  PanelRightIcon,
  RefreshCwIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"

import { OntologyDrawer } from "~/components/stratum/ontology-drawer"
import { OntologyGraphCanvas } from "~/components/stratum/ontology-graph-canvas"
import { OntologyInspector } from "~/components/stratum/ontology-inspector"
import { OntologySourcePanel } from "~/components/stratum/ontology-source-panel"
import { Button } from "~/components/ui/button"
import { useOntologyWorkspace } from "~/hooks/use-ontology-workspace"
import type { SchemaSource } from "~/lib/ontology-api"
import type { OntologySelection } from "~/lib/ontology-graph"

function LoadingCanvas() {
  return (
    <div
      className="relative h-full overflow-hidden bg-wyse-canvas"
      aria-busy="true"
    >
      <div className="absolute top-[24%] left-[12%] h-16 w-44 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />
      <div className="absolute top-[48%] left-[42%] h-16 w-44 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />
      <div className="absolute top-[28%] left-[70%] h-16 w-44 animate-pulse rounded-lg bg-muted motion-reduce:animate-none" />
    </div>
  )
}

export function OntologyWorkspace() {
  const { t } = useTranslation()
  const { state, options, selectSource, retry } = useOntologyWorkspace()
  const [selection, setSelection] = useState<OntologySelection>(null)
  const [sourceOpen, setSourceOpen] = useState(false)
  const [inspectorOpen, setInspectorOpen] = useState(false)
  const sourceButtonRef = useRef<HTMLButtonElement>(null)
  const inspectorButtonRef = useRef<HTMLButtonElement>(null)
  const graph = "graph" in state ? state.graph : undefined
  const schema = "schema" in state ? state.schema : undefined

  useEffect(() => setSelection(null), [state.source])

  useEffect(() => {
    if (!selection || !graph) return
    const exists =
      selection.kind === "node"
        ? graph.nodes.some((node) => node.id === selection.id)
        : graph.edges.some((edge) => edge.id === selection.id)
    if (!exists) setSelection(null)
  }, [graph, selection])

  useEffect(() => {
    const desktopQuery = window.matchMedia("(min-width: 1024px)")
    const closeMobileDrawers = (event: MediaQueryListEvent) => {
      if (!event.matches) return
      setSourceOpen(false)
      setInspectorOpen(false)
    }

    desktopQuery.addEventListener("change", closeMobileDrawers)
    return () => desktopQuery.removeEventListener("change", closeMobileDrawers)
  }, [])

  const handleSourceChange = (source: SchemaSource) => {
    setSourceOpen(false)
    setSelection(null)
    selectSource(source)
  }

  const handleSelectionChange = (next: OntologySelection) => {
    setSelection(next)
    if (next && window.matchMedia("(max-width: 1023px)").matches) {
      setSourceOpen(false)
      setInspectorOpen(true)
    }
  }

  const sourcePanel = (
    <OntologySourcePanel
      source={state.source}
      options={options}
      graph={graph}
      selection={selection}
      disabled={state.phase === "loading"}
      onSourceChange={handleSourceChange}
      onSelectionChange={handleSelectionChange}
    />
  )
  const inspector = (
    <OntologyInspector graph={graph} schema={schema} selection={selection} />
  )

  return (
    <section className="mx-auto h-[100dvh] min-h-[36rem] max-w-[100rem] px-4 pt-24 pb-4 md:px-8 md:pt-28 md:pb-6">
      <div className="grid h-full min-h-0 overflow-hidden border-y border-wyse-line bg-wyse-paper lg:grid-cols-[15rem_minmax(0,1fr)_19rem] lg:border-x">
        <div className="hidden min-h-0 border-r border-wyse-line lg:block">
          {sourcePanel}
        </div>

        <div className="relative min-h-0 overflow-hidden">
          <div className="absolute top-3 left-3 z-20 flex gap-2 lg:hidden">
            <Button
              ref={sourceButtonRef}
              type="button"
              variant="outline"
              className="h-11 text-sm"
              aria-expanded={sourceOpen}
              onClick={() => setSourceOpen(true)}
            >
              <ListTreeIcon aria-hidden="true" />
              {t("ontology.source.index")}
            </Button>
          </div>
          <div className="absolute top-3 right-3 z-20 lg:hidden">
            <Button
              ref={inspectorButtonRef}
              type="button"
              variant="outline"
              className="h-11 text-sm"
              aria-expanded={inspectorOpen}
              onClick={() => setInspectorOpen(true)}
            >
              <PanelRightIcon aria-hidden="true" />
              {t("ontology.inspector.title")}
            </Button>
          </div>

          {state.phase === "demo" ? (
            <div
              role="status"
              className="absolute inset-x-0 top-0 z-10 flex min-h-10 items-center gap-2 border-b border-wyse-line bg-wyse-paper-soft px-3 text-sm"
            >
              <InfoIcon
                className="size-4 text-wyse-action"
                aria-hidden="true"
              />
              <strong>{t("ontology.state.demo")}</strong>
              <span className="text-muted-foreground">
                {t(`ontology.state.${state.demoReason}`)}
              </span>
              <Button
                type="button"
                variant="outline"
                className="ml-auto h-11 text-sm"
                onClick={retry}
              >
                <RefreshCwIcon aria-hidden="true" />
                {t("ontology.state.retry")}
              </Button>
            </div>
          ) : null}

          {state.phase === "loading" ? <LoadingCanvas /> : null}
          {(state.phase === "ready" || state.phase === "demo") && graph ? (
            <OntologyGraphCanvas
              graph={graph}
              selection={selection}
              onSelectionChange={handleSelectionChange}
            />
          ) : null}
          {state.phase === "empty" ? (
            <div className="grid h-full place-items-center bg-wyse-canvas px-6 text-center">
              <div className="max-w-md">
                <h2 className="text-lg font-semibold">
                  {t("ontology.state.emptyTitle")}
                </h2>
                <p className="mt-2 text-sm text-muted-foreground">
                  {t("ontology.state.emptyDescription")}
                </p>
              </div>
            </div>
          ) : null}
          {state.phase === "error" ? (
            <div className="grid h-full place-items-center bg-wyse-canvas px-6 text-center">
              <div className="max-w-md">
                <h2 className="text-lg font-semibold">
                  {t("ontology.state.errorTitle")}
                </h2>
                <p className="mt-2 text-sm text-muted-foreground">
                  {t("ontology.state.errorDescription")}
                </p>
                <Button
                  className="mt-4 h-11 text-sm"
                  type="button"
                  variant="outline"
                  onClick={retry}
                >
                  <RefreshCwIcon aria-hidden="true" />
                  {t("ontology.state.retry")}
                </Button>
              </div>
            </div>
          ) : null}
        </div>

        <div className="hidden min-h-0 border-l border-wyse-line lg:block">
          {inspector}
        </div>
      </div>

      <OntologyDrawer
        open={sourceOpen}
        side="left"
        label={t("ontology.source.index")}
        closeLabel={t("ontology.actions.close")}
        returnFocusRef={sourceButtonRef}
        onOpenChange={setSourceOpen}
      >
        {sourcePanel}
      </OntologyDrawer>
      <OntologyDrawer
        open={inspectorOpen}
        side="right"
        label={t("ontology.inspector.title")}
        closeLabel={t("ontology.actions.close")}
        returnFocusRef={inspectorButtonRef}
        onOpenChange={setInspectorOpen}
      >
        {inspector}
      </OntologyDrawer>
    </section>
  )
}
