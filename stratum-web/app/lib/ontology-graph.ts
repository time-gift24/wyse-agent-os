import dagre from "@dagrejs/dagre"
import { MarkerType, type Edge, type Node } from "@xyflow/react"

import type { Cardinality, OntologyGraph } from "~/lib/ontology-api"

export const ONTOLOGY_NODE_WIDTH = 176
export const ONTOLOGY_NODE_HEIGHT = 68

export type OntologySelection =
  | { kind: "node"; id: string }
  | { kind: "edge"; id: string }
  | null

export type OntologyNodeData = {
  label: string
  propertyCount: number
}

export type OntologyEdgeData = {
  label: string
  cardinality: Cardinality
}

export type OntologyFlowNode = Node<OntologyNodeData, "ontology">
export type OntologyFlowEdge = Edge<OntologyEdgeData>

export function cardinalityLabel(cardinality: Cardinality): string {
  return {
    one_to_one: "1:1",
    one_to_many: "1:N",
    many_to_one: "N:1",
    many_to_many: "N:N",
  }[cardinality]
}

export function createOntologyFlow(graph: OntologyGraph): {
  nodes: OntologyFlowNode[]
  edges: OntologyFlowEdge[]
} {
  const layout = new dagre.graphlib.Graph().setDefaultEdgeLabel(() => ({}))
  layout.setGraph({
    rankdir: "LR",
    ranksep: 96,
    nodesep: 48,
    marginx: 32,
    marginy: 32,
  })

  for (const node of graph.nodes) {
    layout.setNode(node.id, {
      width: ONTOLOGY_NODE_WIDTH,
      height: ONTOLOGY_NODE_HEIGHT,
    })
  }
  for (const edge of graph.edges) layout.setEdge(edge.source, edge.target)
  dagre.layout(layout)

  return {
    nodes: graph.nodes.map((node) => {
      const position = layout.node(node.id)
      return {
        id: node.id,
        type: "ontology",
        position: {
          x: position.x - ONTOLOGY_NODE_WIDTH / 2,
          y: position.y - ONTOLOGY_NODE_HEIGHT / 2,
        },
        data: {
          label: node.label,
          propertyCount: node.property_count,
        },
        draggable: false,
        connectable: false,
        selectable: true,
      }
    }),
    edges: graph.edges.map((edge) => ({
      id: edge.id,
      source: edge.source,
      target: edge.target,
      type: "smoothstep",
      label: `${edge.label} · ${cardinalityLabel(edge.cardinality)}`,
      data: { label: edge.label, cardinality: edge.cardinality },
      selectable: true,
      focusable: true,
      markerEnd: { type: MarkerType.ArrowClosed },
    })),
  }
}
