import { Handle, Position, type NodeProps } from "@xyflow/react"
import { useTranslation } from "react-i18next"

import type { OntologyFlowNode } from "~/lib/ontology-graph"
import { cn } from "~/lib/utils"

export function OntologyNode({ data, selected }: NodeProps<OntologyFlowNode>) {
  const { t } = useTranslation()

  return (
    <div
      className={cn(
        "w-44 rounded-lg border bg-wyse-paper px-3 py-2.5 text-left text-foreground",
        selected ? "border-2 border-wyse-action" : "border-input"
      )}
    >
      <Handle
        type="target"
        position={Position.Left}
        isConnectable={false}
        className="!size-1.5 !border-0 !bg-wyse-action"
      />
      <span className="block text-sm font-semibold">{data.label}</span>
      <span className="mt-0.5 block text-sm text-muted-foreground">
        {t("ontology.node.properties", { count: data.propertyCount })}
      </span>
      <Handle
        type="source"
        position={Position.Right}
        isConnectable={false}
        className="!size-1.5 !border-0 !bg-wyse-action"
      />
    </div>
  )
}
