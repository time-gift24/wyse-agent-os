"use client"

import {
  useEffect,
  useMemo,
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
  "--xy-edge-stroke": "var(--wyse-ink-muted)",
  "--xy-edge-stroke-selected": "var(--wyse-action)",
  "--xy-edge-label-color": "var(--wyse-ink)",
  "--xy-edge-label-background-color": "var(--wyse-canvas)",
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
  const nodes = useMemo(
    () =>
      flow.nodes.map((node) => ({
        ...node,
        className: cn(
          node.className,
          "rounded-lg focus-visible:outline-2! focus-visible:outline-offset-2! focus-visible:outline-ring!"
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
          ? "var(--wyse-action)"
          : "var(--wyse-ink-muted)"

        return {
          ...edge,
          selected,
          style: {
            ...edge.style,
            strokeWidth: selected ? 2.5 : 1.25,
          },
          labelStyle: {
            ...edge.labelStyle,
            fontSize: 14,
            fontWeight: 600,
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
      padding: 0.8,
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
      className="relative h-full min-h-0 w-full bg-wyse-canvas"
      role="region"
      aria-label={t("ontology.canvas.label")}
    >
      <ReactFlow<OntologyFlowNode, OntologyFlowEdge>
        className="[&_.react-flow\_\_edge.selectable:focus-visible_.react-flow\_\_edge-path]:[stroke-width:3px]"
        style={flowStyle}
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.3 }}
        minZoom={0.2}
        maxZoom={2}
        nodesDraggable={false}
        nodesConnectable={false}
        edgesReconnectable={false}
        deleteKeyCode={null}
        elementsSelectable
        nodesFocusable
        edgesFocusable
        defaultMarkerColor="var(--wyse-ink-muted)"
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
          gap={18}
          size={1}
          color="var(--wyse-line-strong)"
        />
      </ReactFlow>
      <div className="absolute bottom-3 left-3 z-10 flex rounded-md border border-wyse-line bg-wyse-paper p-0.5">
        <button
          type="button"
          className="grid size-11 place-items-center rounded-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
          aria-label={t("ontology.canvas.zoomOut")}
          onClick={() => void zoomOut({ duration })}
        >
          <MinusIcon className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          className="grid size-11 place-items-center rounded-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
          aria-label={t("ontology.canvas.zoomIn")}
          onClick={() => void zoomIn({ duration })}
        >
          <PlusIcon className="size-4" aria-hidden="true" />
        </button>
        <button
          type="button"
          className="grid size-11 place-items-center rounded-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring"
          aria-label={t("ontology.canvas.fitView")}
          onClick={() => void fitView({ padding: 0.3, duration })}
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
