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
  if (!selection || !graph || !schema) {
    return (
      <aside className="h-full bg-wyse-paper p-4">
        <h2 className="text-sm font-semibold">
          {t("ontology.inspector.title")}
        </h2>
        <p className="mt-3 text-sm text-muted-foreground">
          {t("ontology.inspector.empty")}
        </p>
      </aside>
    )
  }

  if (selection.kind === "node") {
    const objectType = schema.object_types.find(
      (item) => item.id === selection.id
    )
    if (!objectType) return null
    const relationCount = graph.edges.filter(
      (edge) => edge.source === selection.id || edge.target === selection.id
    ).length
    return (
      <aside className="h-full overflow-y-auto bg-wyse-paper p-4">
        <span className="text-sm text-muted-foreground">
          {t("ontology.inspector.objectType")}
        </span>
        <h2 className="mt-1 text-lg font-semibold">{objectType.name}</h2>
        {objectType.description ? (
          <p className="mt-1 text-sm text-muted-foreground">
            {objectType.description}
          </p>
        ) : null}
        <dl className="my-4 flex gap-6 border-y border-wyse-line py-3 text-sm">
          <div>
            <dt className="text-muted-foreground">
              {t("ontology.inspector.properties")}
            </dt>
            <dd className="font-semibold">{objectType.properties.length}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">
              {t("ontology.inspector.relations")}
            </dt>
            <dd className="font-semibold">{relationCount}</dd>
          </div>
        </dl>
        <h3 className="text-sm font-semibold">
          {t("ontology.inspector.properties")}
        </h3>
        <div className="mt-2">
          {objectType.properties.map((property) => (
            <div
              key={property.id}
              className="grid grid-cols-[minmax(0,1fr)_auto_auto] gap-2 border-t border-wyse-line py-2 text-sm"
            >
              <span className="truncate font-medium">{property.name}</span>
              <span className="text-muted-foreground">
                {property.value_type}
              </span>
              <span
                className={
                  property.required
                    ? "text-wyse-action"
                    : "text-muted-foreground"
                }
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
      </aside>
    )
  }

  const edge = graph.edges.find((item) => item.id === selection.id)
  const linkType = schema.link_types.find((item) => item.id === selection.id)
  if (!edge || !linkType) return null
  const source = schema.object_types.find((item) => item.id === edge.source)
  const target = schema.object_types.find((item) => item.id === edge.target)

  return (
    <aside className="h-full overflow-y-auto bg-wyse-paper p-4">
      <span className="text-sm text-muted-foreground">
        {t("ontology.inspector.linkType")}
      </span>
      <h2 className="mt-1 text-lg font-semibold">{linkType.name}</h2>
      {linkType.description ? (
        <p className="mt-1 text-sm text-muted-foreground">
          {linkType.description}
        </p>
      ) : null}
      <dl className="mt-4 space-y-3 border-t border-wyse-line pt-3 text-sm">
        <div>
          <dt className="text-muted-foreground">
            {t("ontology.inspector.source")}
          </dt>
          <dd className="font-medium">{source?.name ?? edge.source}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">
            {t("ontology.inspector.target")}
          </dt>
          <dd className="font-medium">{target?.name ?? edge.target}</dd>
        </div>
        <div>
          <dt className="text-muted-foreground">
            {t("ontology.inspector.cardinality")}
          </dt>
          <dd className="font-medium">{cardinalityLabel(edge.cardinality)}</dd>
        </div>
      </dl>
    </aside>
  )
}
