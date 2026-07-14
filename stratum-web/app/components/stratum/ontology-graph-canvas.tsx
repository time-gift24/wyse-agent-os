"use client"

import {
  useEffect,
  useMemo,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react"
import {
  Background,
  BackgroundVariant,
  ReactFlow,
  ReactFlowProvider,
  useReactFlow,
} from "@xyflow/react"
import "@xyflow/react/dist/style.css"
import { MinusIcon, PlusIcon, ScanIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { useReducedMotion } from "motion/react"

import { OntologyNode } from "~/components/stratum/ontology-node"
import type { OntologyGraph } from "~/lib/ontology-api"
import {
  createOntologyFlow,
  type OntologyFlowEdge,
  type OntologyFlowNode,
  type OntologySelection,
} from "~/lib/ontology-graph"
import { cn } from "~/lib/utils"

const nodeTypes = { ontology: OntologyNode }
const flowStyle = {
  "--xy-edge-stroke": "var(--stratum-ink-muted)",
  "--xy-edge-stroke-selected": "var(--stratum-action)",
  "--xy-edge-label-color": "var(--stratum-ink)",
  "--xy-edge-label-background-color": "var(--stratum-canvas)",
} as CSSProperties

type OntologyGraphCanvasProps = {
  graph: OntologyGraph
  selection: OntologySelection
  onSelectionChange(selection: OntologySelection): void
}

function CanvasInner({
  graph,
  selection,
  onSelectionChange,
}: OntologyGraphCanvasProps) {
  const { t } = useTranslation()
  const reduceMotion = useReducedMotion()
  const flow = useMemo(() => createOntologyFlow(graph), [graph])
  const [isDesktop, setIsDesktop] = useState(false)

  useEffect(() => {
    const check = () => setIsDesktop(window.innerWidth >= 1024)
    check()
    window.addEventListener("resize", check)
    return () => window.removeEventListener("resize", check)
  }, [])

  const canvasPadding = isDesktop ? 0.28 : 0.15
  const nodes = useMemo(
    () =>
      flow.nodes.map((node) => ({
        ...node,
        className: cn(
          node.className,
          "transition-all duration-200 focus-visible:outline-2! focus-visible:outline-offset-2! focus-visible:outline-ring!"
        ),
        selected: selection?.kind === "node" && selection.id === node.id,
      })),
    [flow.nodes, selection]
  )
  const edges = useMemo(
    () =>
      flow.edges.map((edge) => {
        const selected = selection?.kind === "edge" && selection.id === edge.id
        const markerColor = selected
          ? "var(--stratum-action)"
          : "var(--stratum-ink-muted)"

        return {
          ...edge,
          selected,
          labelStyle: {
            ...edge.labelStyle,
            fontSize: 13,
            fontWeight: 600,
            padding: "2px 6px",
            borderRadius: "2px",
          },
          markerEnd:
            typeof edge.markerEnd === "object"
              ? { ...edge.markerEnd, color: markerColor }
              : edge.markerEnd,
        }
      }),
    [flow.edges, selection]
  )
  const { fitView, zoomIn, zoomOut } = useReactFlow<
    OntologyFlowNode,
    OntologyFlowEdge
  >()

  useEffect(() => {
    if (!selection) return
    const targets =
      selection.kind === "node"
        ? nodes.filter((node) => node.id === selection.id)
        : edges
            .filter((edge) => edge.id === selection.id)
            .flatMap((edge) =>
              nodes.filter(
                (node) => node.id === edge.source || node.id === edge.target
              )
            )
    if (targets.length === 0) return
    void fitView({
      nodes: targets,
      padding: isDesktop ? 0.35 : 0.15,
      duration: reduceMotion ? 0 : 180,
    })
  }, [edges, fitView, nodes, reduceMotion, selection])

  const duration = reduceMotion ? 0 : 180

  const handleElementKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " " && event.key !== "Escape") {
      return
    }

    const target = event.target
    if (!(target instanceof Element)) return

    const element = target.closest(".react-flow__node, .react-flow__edge")
    const id = element?.getAttribute("data-id")
    if (!element || !id) return

    if (event.key === "Escape") {
      onSelectionChange(null)
      return
    }

    if (event.key === " ") event.preventDefault()
    onSelectionChange({
      kind: element.classList.contains("react-flow__node") ? "node" : "edge",
      id,
    })
  }

  return (
    <div
      className="relative h-full min-h-0 w-full"
      role="region"
      aria-label={t("ontology.canvas.label")}
    >
      <ReactFlow<OntologyFlowNode, OntologyFlowEdge>
        className="ontology-graph-canvas"
        style={flowStyle}
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: canvasPadding }}
        minZoom={0.2}
        maxZoom={2}
        nodesDraggable={false}
        nodesConnectable={false}
        edgesReconnectable={false}
        deleteKeyCode={null}
        elementsSelectable
        nodesFocusable
        edgesFocusable
        defaultMarkerColor="var(--stratum-ink-muted)"
        onKeyDownCapture={handleElementKeyDown}
        onNodeClick={(_, node) =>
          onSelectionChange({ kind: "node", id: node.id })
        }
        onEdgeClick={(_, edge) =>
          onSelectionChange({ kind: "edge", id: edge.id })
        }
        onPaneClick={() => onSelectionChange(null)}
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={28}
          size={1.5}
          color="var(--stratum-line)"
        />
      </ReactFlow>
      <div className="absolute bottom-6 left-1/2 z-10 flex -translate-x-1/2 rounded-2xl border border-stratum-line bg-stratum-paper p-1 shadow-stratum-soft">
        <button
          type="button"
          className="grid size-10 place-items-center rounded-xl text-muted-foreground transition-colors hover:bg-stratum-paper-soft hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          aria-label={t("ontology.canvas.zoomOut")}
          onClick={() => void zoomOut({ duration })}
        >
          <MinusIcon className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          className="grid size-10 place-items-center rounded-xl text-muted-foreground transition-colors hover:bg-stratum-paper-soft hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          aria-label={t("ontology.canvas.zoomIn")}
          onClick={() => void zoomIn({ duration })}
        >
          <PlusIcon className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          className="grid size-10 place-items-center rounded-xl text-muted-foreground transition-colors hover:bg-stratum-paper-soft hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          aria-label={t("ontology.canvas.fitView")}
          onClick={() => void fitView({ padding: canvasPadding, duration })}
        >
          <ScanIcon className="size-4" aria-hidden="true" />
        </button>
      </div>
    </div>
  )
}

export function OntologyGraphCanvas(props: OntologyGraphCanvasProps) {
  return (
    <ReactFlowProvider>
      <CanvasInner {...props} />
    </ReactFlowProvider>
  )
}
