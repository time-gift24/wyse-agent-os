"use client"

import { useEffect, useRef, useState } from "react"
import {
  InfoIcon,
  ListTreeIcon,
  PanelRightIcon,
  RefreshCwIcon,
} from "lucide-react"
import { useTranslation } from "react-i18next"
import { AnimatePresence, motion, useReducedMotion } from "motion/react"

import { OntologyDrawer } from "~/components/stratum/ontology-drawer"
import { OntologyGraphCanvas } from "~/components/stratum/ontology-graph-canvas"
import { OntologyInspector } from "~/components/stratum/ontology-inspector"
import { OntologySourcePanel } from "~/components/stratum/ontology-source-panel"
import { Button } from "~/components/ui/button"
import { useOntologyWorkspace } from "~/hooks/use-ontology-workspace"
import type { SchemaSource } from "~/lib/ontology-api"
import type { OntologySelection } from "~/lib/ontology-graph"
import { cn } from "~/lib/utils"

function LoadingCanvas({ label }: { label: string }) {
  return (
    <div
      className="relative h-full overflow-hidden"
      role="status"
      aria-live="polite"
      aria-busy="true"
    >
      <span className="sr-only">{label}</span>
      <div className="absolute top-[24%] left-[12%] h-16 w-44 animate-pulse rounded-lg bg-stratum-paper-soft motion-reduce:animate-none" />
      <div className="absolute top-[48%] left-[42%] h-16 w-44 animate-pulse rounded-lg bg-stratum-paper-soft motion-reduce:animate-none" />
      <div className="absolute top-[28%] left-[70%] h-16 w-44 animate-pulse rounded-lg bg-stratum-paper-soft motion-reduce:animate-none" />
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
  const reduceMotion = useReducedMotion()

  useEffect(() => setSelection(null), [state.source])

  useEffect(() => {
    if (!selection || !graph) return
    const exists =
      selection.kind === "node"
        ? graph.nodes.some((node) => node.id === selection.id)
        : graph.edges.some((edge) => edge.id === selection.id)
    if (!exists) setSelection(null)
  }, [graph, selection])

  const handleSourceChange = (source: SchemaSource) => {
    setSourceOpen(false)
    setSelection(null)
    selectSource(source)
  }

  const handleSelectionChange = (next: OntologySelection) => {
    setSelection(next)
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

  const floatingPanelVariants = {
    hidden: {
      scale: reduceMotion ? 1 : 0.97,
      opacity: reduceMotion ? 1 : 0,
      y: reduceMotion ? 0 : -8,
    },
    visible: { scale: 1, y: 0, opacity: 1 },
  }

  return (
    <section className="relative min-h-[100dvh] w-full pt-24 md:pt-28">
      {/* Full screen Canvas Background */}
      <div className="fixed inset-0 -z-10 h-full w-full bg-stratum-canvas" />

      {/* Demo Status Banner */}
      {state.phase === "demo" ? (
        <motion.div
          key="demo-banner"
          initial={false}
          variants={floatingPanelVariants}
          animate="visible"
          transition={{ duration: reduceMotion ? 0 : 0.25, ease: "easeOut" }}
          className="fixed inset-x-0 top-28 z-40 px-4 md:px-8"
        >
          <div
            role="status"
            className="mx-auto flex max-w-3xl flex-wrap items-center gap-x-2 gap-y-1 rounded-2xl border border-stratum-line bg-stratum-paper/90 px-4 py-3 shadow-stratum-soft backdrop-blur-sm"
          >
            <div className="flex h-6 w-6 items-center justify-center rounded-full bg-stratum-action/10">
              <InfoIcon className="size-3.5 text-stratum-action" aria-hidden="true" />
            </div>
            <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2">
              <strong className="text-sm font-semibold text-foreground">
                {t("ontology.state.demo")}
              </strong>
              <span className="text-sm text-muted-foreground">
                {t(`ontology.state.${state.demoReason}`)}
              </span>
            </div>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={retry}
            >
              <RefreshCwIcon className="mr-1 size-3.5" aria-hidden="true" />
              {t("ontology.state.retry")}
            </Button>
          </div>
        </motion.div>
      ) : null}

      {/* Mobile Toggle Buttons */}
      <div className="fixed top-28 z-40 flex w-full items-center justify-between px-4 md:px-8 lg:hidden">
        <Button
          ref={sourceButtonRef}
          type="button"
          variant="outline"
          size="sm"
          aria-expanded={sourceOpen}
          onClick={() => setSourceOpen(true)}
          className="shadow-stratum-soft"
        >
          <ListTreeIcon className="mr-1.5 size-3.5" aria-hidden="true" />
          {t("ontology.source.index")}
        </Button>
        <Button
          ref={inspectorButtonRef}
          type="button"
          variant="outline"
          size="sm"
          aria-expanded={inspectorOpen}
          onClick={() => setInspectorOpen(true)}
          className="shadow-stratum-soft"
        >
          <PanelRightIcon className="mr-1.5 size-3.5" aria-hidden="true" />
          {t("ontology.inspector.title")}
        </Button>
      </div>

      {/* Desktop Floating Source Panel */}
      <AnimatePresence initial={false}>
        <div className="hidden lg:block">
          <motion.aside
            initial="hidden"
            animate="visible"
            variants={floatingPanelVariants}
            transition={{ duration: reduceMotion ? 0 : 0.3, ease: "easeOut", delay: 0.1 }}
            className="fixed left-12 top-32 z-30 w-60 max-h-[calc(100dvh-10rem)]"
          >
            <div className="flex flex-col overflow-hidden rounded-2xl border border-stratum-line bg-stratum-paper shadow-stratum-soft">
              {sourcePanel}
            </div>
          </motion.aside>
        </div>
      </AnimatePresence>

      {/* Desktop Floating Inspector Panel */}
      <AnimatePresence initial={false}>
        <div className="hidden lg:block">
          <motion.aside
            initial="hidden"
            animate="visible"
            variants={floatingPanelVariants}
            transition={{ duration: reduceMotion ? 0 : 0.3, ease: "easeOut", delay: 0.15 }}
            className="fixed right-6 top-32 z-30 w-60 max-h-[calc(100dvh-10rem)]"
          >
            <div className="flex flex-col overflow-hidden rounded-2xl border border-stratum-line bg-stratum-paper shadow-stratum-soft">
              {inspector}
            </div>
          </motion.aside>
        </div>
      </AnimatePresence>

      {/* Graph Canvas - Full Screen */}
      <div className="relative h-[calc(100dvh-7rem)] w-full">
        {state.phase === "loading" ? (
          <LoadingCanvas label={t("ontology.state.loading")} />
        ) : null}
        {(state.phase === "ready" || state.phase === "demo") && graph ? (
          <OntologyGraphCanvas
            graph={graph}
            selection={selection}
            onSelectionChange={handleSelectionChange}
          />
        ) : null}
        {state.phase === "empty" ? (
          <div
            className="grid h-full place-items-center px-6 text-center"
            role="status"
            aria-live="polite"
          >
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
          <div
            className="grid h-full place-items-center px-6 text-center"
            role="alert"
          >
            <div className="max-w-md">
              <h2 className="text-lg font-semibold">
                {t("ontology.state.errorTitle")}
              </h2>
              <p className="mt-2 text-sm text-muted-foreground">
                {t("ontology.state.errorDescription")}
              </p>
              <Button
                className="mt-4"
                type="button"
                variant="outline"
                onClick={retry}
              >
                <RefreshCwIcon className="mr-2 size-4" aria-hidden="true" />
                {t("ontology.state.retry")}
              </Button>
            </div>
          </div>
        ) : null}
      </div>

      {/* Mobile Drawers */}
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
