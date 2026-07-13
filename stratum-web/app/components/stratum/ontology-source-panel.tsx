"use client"

import { useEffect, useMemo, useState, type KeyboardEvent } from "react"
import { SearchIcon } from "lucide-react"
import { useTranslation } from "react-i18next"

import { Button } from "~/components/ui/button"
import { Input } from "~/components/ui/input"
import type {
  OntologyGraph,
  SchemaSource,
  SourceOptions,
} from "~/lib/ontology-api"
import { cardinalityLabel, type OntologySelection } from "~/lib/ontology-graph"
import { cn } from "~/lib/utils"

type SourceKind = SchemaSource["kind"]

type OntologySourcePanelProps = {
  source: SchemaSource
  options: SourceOptions
  graph?: OntologyGraph
  selection: OntologySelection
  disabled: boolean
  onSourceChange(source: SchemaSource): void
  onSelectionChange(selection: OntologySelection): void
}

export function OntologySourcePanel({
  source,
  options,
  graph,
  selection,
  disabled,
  onSourceChange,
  onSelectionChange,
}: OntologySourcePanelProps) {
  const { t } = useTranslation()
  const [kind, setKind] = useState<SourceKind>(source.kind)
  const [tag, setTag] = useState(source.kind === "tag" ? source.name : "online")
  const [query, setQuery] = useState("")
  const [nodeFocusId, setNodeFocusId] = useState<string | null>(
    selection?.kind === "node" ? selection.id : null
  )
  const [edgeFocusId, setEdgeFocusId] = useState<string | null>(
    selection?.kind === "edge" ? selection.id : null
  )

  useEffect(() => {
    setKind(source.kind)
    if (source.kind === "tag") setTag(source.name)
  }, [source])

  useEffect(() => {
    if (selection?.kind === "node") setNodeFocusId(selection.id)
    if (selection?.kind === "edge") setEdgeFocusId(selection.id)
  }, [selection])

  const normalizedQuery = query.trim().toLocaleLowerCase()
  const nodes = useMemo(
    () =>
      (graph?.nodes ?? []).filter((node) =>
        node.label.toLocaleLowerCase().includes(normalizedQuery)
      ),
    [graph?.nodes, normalizedQuery]
  )
  const edges = useMemo(
    () =>
      (graph?.edges ?? []).filter((edge) =>
        edge.label.toLocaleLowerCase().includes(normalizedQuery)
      ),
    [graph?.edges, normalizedQuery]
  )

  const nodeTabStopId = nodes.some((node) => node.id === nodeFocusId)
    ? nodeFocusId
    : nodes[0]?.id
  const edgeTabStopId = edges.some((edge) => edge.id === edgeFocusId)
    ? edgeFocusId
    : edges[0]?.id

  const handleListboxKeyDown = (
    event: KeyboardEvent<HTMLDivElement>,
    onFocusChange: (id: string) => void
  ) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return
    const items = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>(
        '[role="option"]:not(:disabled)'
      )
    )
    if (items.length === 0) return
    const current = items.indexOf(document.activeElement as HTMLButtonElement)
    const next =
      current === -1
        ? event.key === "ArrowDown"
          ? 0
          : items.length - 1
        : event.key === "ArrowDown"
          ? (current + 1) % items.length
          : (current - 1 + items.length) % items.length
    const nextItem = items[next]
    const nextId = nextItem?.dataset.optionId
    if (!nextItem || !nextId) return
    event.preventDefault()
    onFocusChange(nextId)
    nextItem.focus()
  }

  return (
    <aside className="flex h-full min-h-0 flex-col bg-wyse-paper p-4">
      <h2 className="text-sm font-semibold">{t("ontology.source.title")}</h2>
      <div className="mt-3 grid grid-cols-3 rounded-md bg-muted p-1">
        {(["tag", "draft", "revision"] as const).map((value) => (
          <button
            key={value}
            type="button"
            className={cn(
              "h-11 rounded-sm border border-transparent px-2 text-sm focus-visible:outline-2 focus-visible:outline-ring",
              kind === value
                ? "border-input bg-wyse-paper text-foreground"
                : "text-muted-foreground hover:text-foreground"
            )}
            aria-pressed={kind === value}
            onClick={() => setKind(value)}
          >
            {t(`ontology.source.${value}`)}
          </button>
        ))}
      </div>

      {kind === "tag" ? (
        <form
          className="mt-2 flex gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            const name = tag.trim()
            if (name) onSourceChange({ kind: "tag", name })
          }}
        >
          <Input
            value={tag}
            onChange={(event) => setTag(event.target.value)}
            aria-label={t("ontology.source.tagName")}
            disabled={disabled}
            className="h-11 md:text-sm"
          />
          <Button
            type="submit"
            variant="outline"
            className="h-11 text-sm"
            disabled={disabled || !tag.trim()}
          >
            {t("ontology.source.load")}
          </Button>
        </form>
      ) : (
        <select
          className="mt-2 h-11 w-full rounded-md border border-input bg-wyse-paper px-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/30"
          value={
            source.kind === kind
              ? source.kind === "revision"
                ? source.id
                : source.name
              : ""
          }
          disabled={disabled}
          aria-label={t(`ontology.source.${kind}`)}
          onChange={(event) => {
            const value = event.target.value
            if (!value) return
            onSourceChange(
              kind === "revision" ? { kind, id: value } : { kind, name: value }
            )
          }}
        >
          <option value="">{t("ontology.source.choose")}</option>
          {(kind === "draft" ? options.drafts : options.revisions).map(
            (option) => {
              const value = "id" in option ? option.id : option.name
              return (
                <option key={value} value={value}>
                  {value}
                </option>
              )
            }
          )}
        </select>
      )}

      <div className="relative mt-3">
        <SearchIcon
          className="pointer-events-none absolute top-1/2 left-2 size-4 -translate-y-1/2 text-muted-foreground"
          aria-hidden="true"
        />
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("ontology.source.search")}
          aria-label={t("ontology.source.search")}
          className="h-11 pl-8 md:text-sm"
        />
      </div>

      <nav
        className="mt-3 min-h-0 flex-1 overflow-y-auto"
        aria-label={t("ontology.source.index")}
      >
        <div className="flex items-center justify-between border-b border-wyse-line py-2 text-sm font-semibold">
          <span>{t("ontology.source.objectTypes")}</span>
          <span className="text-muted-foreground">{nodes.length}</span>
        </div>
        <div
          role="listbox"
          aria-label={t("ontology.source.objectTypes")}
          onKeyDown={(event) => handleListboxKeyDown(event, setNodeFocusId)}
        >
          {nodes.map((node) => {
            const selected =
              selection?.kind === "node" && selection.id === node.id
            return (
              <button
                key={node.id}
                type="button"
                role="option"
                aria-selected={selected}
                data-option-id={node.id}
                tabIndex={node.id === nodeTabStopId ? 0 : -1}
                className={cn(
                  "mt-1 flex min-h-11 w-full items-center gap-2 rounded-md px-2 text-left text-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring",
                  selected && "bg-wyse-action/10 font-semibold"
                )}
                onClick={() => {
                  setNodeFocusId(node.id)
                  onSelectionChange({ kind: "node", id: node.id })
                }}
              >
                <span className="size-2 rounded-sm border border-wyse-action" />
                <span className="truncate">{node.label}</span>
              </button>
            )
          })}
        </div>

        <div className="mt-3 flex items-center justify-between border-b border-wyse-line py-2 text-sm font-semibold">
          <span>{t("ontology.source.linkTypes")}</span>
          <span className="text-muted-foreground">{edges.length}</span>
        </div>
        <div
          role="listbox"
          aria-label={t("ontology.source.linkTypes")}
          onKeyDown={(event) => handleListboxKeyDown(event, setEdgeFocusId)}
        >
          {edges.map((edge) => {
            const selected =
              selection?.kind === "edge" && selection.id === edge.id
            return (
              <button
                key={edge.id}
                type="button"
                role="option"
                aria-selected={selected}
                data-option-id={edge.id}
                tabIndex={edge.id === edgeTabStopId ? 0 : -1}
                className={cn(
                  "mt-1 flex min-h-11 w-full items-center gap-2 rounded-md px-2 text-left text-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring",
                  selected && "bg-wyse-action/10 font-semibold"
                )}
                onClick={() => {
                  setEdgeFocusId(edge.id)
                  onSelectionChange({ kind: "edge", id: edge.id })
                }}
              >
                <span className="h-px w-3 bg-wyse-action" />
                <span className="min-w-0 flex-1 truncate">{edge.label}</span>
                <span className="text-muted-foreground">
                  {cardinalityLabel(edge.cardinality)}
                </span>
              </button>
            )
          })}
        </div>
      </nav>
    </aside>
  )
}
