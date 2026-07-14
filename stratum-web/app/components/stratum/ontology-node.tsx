import { Handle, Position, type NodeProps } from "@xyflow/react"
import { useTranslation } from "react-i18next"

import type { OntologyFlowNode } from "~/lib/ontology-graph"
import { cn } from "~/lib/utils"

export function OntologyNode({ data, selected }: NodeProps<OntologyFlowNode>) {
  const { t } = useTranslation()

  return (
    <div
      className={cn(
        "w-44 border bg-stratum-paper px-3 py-2 text-left text-foreground shadow-sm transition-all duration-200",
        selected
          ? "border-2 border-stratum-action shadow-md shadow-stratum-action/10"
          : "border-stratum-line hover:border-stratum-action/50"
      )}
      style={{ borderRadius: "0.375rem" }}
    >
      <Handle
        type="target"
        position={Position.Left}
        isConnectable={false}
        className={cn(
          "!size-2.5 !rounded-none !border-0 !shadow-none transition-all duration-200",
          selected ? "!bg-stratum-action" : "!bg-stratum-ink-muted"
        )}
      />
      <span className="block text-sm font-semibold">{data.label}</span>
      <span className="mt-0.5 block text-xs text-muted-foreground">
        {t("ontology.node.properties", { count: data.propertyCount })}
      </span>
      <Handle
        type="source"
        position={Position.Right}
        isConnectable={false}
        className={cn(
          "!size-2.5 !rounded-none !border-0 !shadow-none transition-all duration-200",
          selected ? "!bg-stratum-action" : "!bg-stratum-ink-muted"
        )}
      />
    </div>
  )
}
