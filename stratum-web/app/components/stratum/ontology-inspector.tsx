import { useTranslation } from "react-i18next"

import type { OntologyGraph, SchemaDocument } from "~/lib/ontology-api"
import { cardinalityLabel, type OntologySelection } from "~/lib/ontology-graph"

type OntologyInspectorProps = {
  graph?: OntologyGraph
  schema?: SchemaDocument
  selection: OntologySelection
}

export function OntologyInspector({
  graph,
  schema,
  selection,
}: OntologyInspectorProps) {
  const { t } = useTranslation()
  const emptyInspector = (
    <div className="flex h-full flex-col px-4 py-4">
      <h2 className="text-xs font-bold uppercase tracking-widest text-muted-foreground">
        {t("ontology.inspector.title")}
      </h2>
      <p className="mt-4 text-sm text-muted-foreground">
        {t("ontology.inspector.empty")}
      </p>
    </div>
  )
  if (!selection || !graph || !schema) return emptyInspector

  if (selection.kind === "node") {
    const objectType = schema.object_types.find(
      (item) => item.id === selection.id
    )
    if (!objectType) return emptyInspector
    const relationCount = graph.edges.filter(
      (edge) => edge.source === selection.id || edge.target === selection.id
    ).length
    return (
      <div className="flex h-full min-h-0 flex-col px-4 py-4">
        <span className="text-xs font-bold uppercase tracking-widest text-muted-foreground">
          {t("ontology.inspector.objectType")}
        </span>
        <h2 className="mt-1 text-base font-semibold text-foreground">
          {objectType.name}
        </h2>
        {objectType.description ? (
          <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
            {objectType.description}
          </p>
        ) : null}

        <dl className="mt-4 grid grid-cols-2 gap-4 border-t border-stratum-line pt-3 text-sm">
          <div>
            <dt className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {t("ontology.inspector.properties")}
            </dt>
            <dd className="mt-0.5 text-lg font-bold">{objectType.properties.length}</dd>
          </div>
          <div>
            <dt className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
              {t("ontology.inspector.relations")}
            </dt>
            <dd className="mt-0.5 text-lg font-bold">{relationCount}</dd>
          </div>
        </dl>

        <h3 className="mt-4 text-xs font-bold uppercase tracking-widest text-muted-foreground">
          {t("ontology.inspector.properties")}
        </h3>
        <div className="mt-2 min-h-0 flex-1 overflow-y-auto -mx-1">
          {objectType.properties.map((property, index) => (
            <div
              key={property.id}
              className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-2 border-t border-stratum-line/50 px-1 py-2 text-sm"
            >
              <span className="truncate font-medium">{property.name}</span>
              <span className="text-xs text-muted-foreground">
                {property.value_type}
              </span>
              <span
                className={cn(
                  "text-xs font-medium",
                  property.required
                    ? "text-stratum-action"
                    : "text-muted-foreground"
                )}
              >
                {t(
                  property.required
                    ? "ontology.inspector.required"
                    : "ontology.inspector.optional"
                )}
              </span>
            </div>
          ))}
        </div>
      </div>
    )
  }

  const edge = graph.edges.find((item) => item.id === selection.id)
  const linkType = schema.link_types.find((item) => item.id === selection.id)
  if (!edge || !linkType) return emptyInspector
  const source = schema.object_types.find((item) => item.id === edge.source)
  const target = schema.object_types.find((item) => item.id === edge.target)

  return (
    <div className="flex h-full min-h-0 flex-col px-4 py-4">
      <span className="text-xs font-bold uppercase tracking-widest text-muted-foreground">
        {t("ontology.inspector.linkType")}
      </span>
      <h2 className="mt-1 text-base font-semibold text-foreground">
        {linkType.name}
      </h2>
      {linkType.description ? (
        <p className="mt-2 text-sm leading-relaxed text-muted-foreground">
          {linkType.description}
        </p>
      ) : null}

      <dl className="mt-4 space-y-3 border-t border-stratum-line pt-3 text-sm">
        <div>
          <dt className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            {t("ontology.inspector.source")}
          </dt>
          <dd className="mt-0.5 font-medium">{source?.name ?? edge.source}</dd>
        </div>
        <div>
          <dt className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            {t("ontology.inspector.target")}
          </dt>
          <dd className="mt-0.5 font-medium">{target?.name ?? edge.target}</dd>
        </div>
        <div>
          <dt className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
            {t("ontology.inspector.cardinality")}
          </dt>
          <dd className="mt-0.5 font-bold text-stratum-action">
            {cardinalityLabel(edge.cardinality)}
          </dd>
        </div>
      </dl>
    </div>
  )
}

function cn(...classes: (string | boolean)[]) {
  return classes.filter(Boolean).join(" ")
}
