export type ValueType =
  | "string"
  | "integer"
  | "number"
  | "boolean"
  | "datetime"
  | "json"

export type Cardinality =
  | "one_to_one"
  | "one_to_many"
  | "many_to_one"
  | "many_to_many"

export type PropertyType = {
  id: string
  name: string
  description: string
  value_type: ValueType
  required: boolean
}

export type ObjectType = {
  id: string
  name: string
  description: string
  properties: readonly PropertyType[]
}

export type LinkType = {
  id: string
  name: string
  description: string
  source_object_type_id: string
  target_object_type_id: string
  cardinality: Cardinality
}

export type SchemaDocument = {
  schema_version: number
  object_types: readonly ObjectType[]
  link_types: readonly LinkType[]
}

export type SchemaSource =
  | { kind: "tag"; name: string }
  | { kind: "draft"; name: string }
  | { kind: "revision"; id: string }

export type GraphNode = {
  id: string
  label: string
  property_count: number
}

export type GraphEdge = {
  id: string
  label: string
  source: string
  target: string
  cardinality: Cardinality
}

export type OntologyGraph = {
  schema_ref: { kind: SchemaSource["kind"]; name: string }
  nodes: readonly GraphNode[]
  edges: readonly GraphEdge[]
}

export type DraftResponse = {
  name: string
  schema: SchemaDocument
  digest: string
}

export type RevisionResponse = {
  id: string
  schema: SchemaDocument
}

export type SourceOptions = {
  drafts: readonly DraftResponse[]
  revisions: readonly RevisionResponse[]
}

const valueTypes = new Set<ValueType>([
  "string",
  "integer",
  "number",
  "boolean",
  "datetime",
  "json",
])

const cardinalities = new Set<Cardinality>([
  "one_to_one",
  "one_to_many",
  "many_to_one",
  "many_to_many",
])

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null

const isString = (value: unknown): value is string => typeof value === "string"

const isPropertyType = (value: unknown): value is PropertyType =>
  isRecord(value) &&
  isString(value.id) &&
  isString(value.name) &&
  isString(value.description) &&
  valueTypes.has(value.value_type as ValueType) &&
  typeof value.required === "boolean"

const isObjectType = (value: unknown): value is ObjectType =>
  isRecord(value) &&
  isString(value.id) &&
  isString(value.name) &&
  isString(value.description) &&
  Array.isArray(value.properties) &&
  value.properties.every(isPropertyType)

const isLinkType = (value: unknown): value is LinkType =>
  isRecord(value) &&
  isString(value.id) &&
  isString(value.name) &&
  isString(value.description) &&
  isString(value.source_object_type_id) &&
  isString(value.target_object_type_id) &&
  cardinalities.has(value.cardinality as Cardinality)

const isSchemaDocument = (value: unknown): value is SchemaDocument =>
  isRecord(value) &&
  value.schema_version === 1 &&
  Array.isArray(value.object_types) &&
  value.object_types.every(isObjectType) &&
  Array.isArray(value.link_types) &&
  value.link_types.every(isLinkType)

const isGraphNode = (value: unknown): value is GraphNode =>
  isRecord(value) &&
  isString(value.id) &&
  isString(value.label) &&
  Number.isSafeInteger(value.property_count) &&
  Number(value.property_count) >= 0

const isGraphEdge = (value: unknown): value is GraphEdge =>
  isRecord(value) &&
  isString(value.id) &&
  isString(value.label) &&
  isString(value.source) &&
  isString(value.target) &&
  cardinalities.has(value.cardinality as Cardinality)

const isOntologyGraph = (value: unknown): value is OntologyGraph =>
  isRecord(value) &&
  isRecord(value.schema_ref) &&
  ["tag", "draft", "revision"].includes(String(value.schema_ref.kind)) &&
  isString(value.schema_ref.name) &&
  Array.isArray(value.nodes) &&
  value.nodes.every(isGraphNode) &&
  Array.isArray(value.edges) &&
  value.edges.every(isGraphEdge)

const isDraftResponse = (value: unknown): value is DraftResponse =>
  isRecord(value) &&
  isString(value.name) &&
  isString(value.digest) &&
  isSchemaDocument(value.schema)

const isRevisionResponse = (value: unknown): value is RevisionResponse =>
  isRecord(value) && isString(value.id) && isSchemaDocument(value.schema)

type OntologyErrorBody = {
  error?: unknown
  message?: unknown
  diagnostics?: unknown
}

export class OntologyApiError extends Error {
  constructor(
    readonly code: string,
    readonly status: number,
    message: string,
    readonly diagnostics: readonly string[] = []
  ) {
    super(message)
    this.name = "OntologyApiError"
  }
}

const invalidResponse = () =>
  new OntologyApiError(
    "invalid_response",
    0,
    "ontology API returned an invalid response"
  )

async function responseError(response: Response): Promise<OntologyApiError> {
  try {
    const value: unknown = await response.json()
    const body = isRecord(value) ? (value as OntologyErrorBody) : undefined
    const code = typeof body?.error === "string" ? body.error : "http_error"
    const message =
      typeof body?.message === "string" ? body.message : "request failed"
    const diagnostics = Array.isArray(body?.diagnostics)
      ? body.diagnostics.filter(isString)
      : []
    return new OntologyApiError(code, response.status, message, diagnostics)
  } catch {
    return new OntologyApiError("http_error", response.status, "request failed")
  }
}

function sourceSearch(source: SchemaSource): string {
  const search = new URLSearchParams()
  if (source.kind === "revision") search.set("revision", source.id)
  else search.set(source.kind, source.name)
  return search.toString()
}

export type OntologyApi = {
  listSources(signal?: AbortSignal): Promise<SourceOptions>
  load(
    source: SchemaSource,
    signal?: AbortSignal
  ): Promise<{ graph: OntologyGraph; schema: SchemaDocument }>
}

export function createOntologyApi(options: {
  baseUrl: string
  fetcher?: typeof fetch
}): OntologyApi {
  const baseUrl = options.baseUrl.replace(/\/$/, "")
  const fetcher = options.fetcher ?? fetch

  const request = async <T>(
    path: string,
    validate: (value: unknown) => value is T,
    signal?: AbortSignal
  ): Promise<T> => {
    const response = await fetcher(`${baseUrl}${path}`, { signal })
    if (!response.ok) throw await responseError(response)
    let value: unknown
    try {
      value = await response.json()
    } catch {
      throw invalidResponse()
    }
    if (!validate(value)) throw invalidResponse()
    return value
  }

  const graph = (source: SchemaSource, signal?: AbortSignal) =>
    request(
      `/v1/ontology/graph?${sourceSearch(source)}`,
      isOntologyGraph,
      signal
    )

  const draft = (name: string, signal?: AbortSignal) =>
    request(
      `/v1/ontology/drafts/${encodeURIComponent(name)}`,
      isDraftResponse,
      signal
    )

  const revision = (id: string, signal?: AbortSignal) =>
    request(
      `/v1/ontology/revisions/${encodeURIComponent(id)}`,
      isRevisionResponse,
      signal
    )

  const tagRevision = async (name: string, signal?: AbortSignal) => {
    const tag = await request(
      `/v1/ontology/tags/${encodeURIComponent(name)}`,
      (value): value is { name: string; revision_id: string } =>
        isRecord(value) && isString(value.name) && isString(value.revision_id),
      signal
    )
    return revision(tag.revision_id, signal)
  }

  return {
    async listSources(signal) {
      const [drafts, revisions] = await Promise.all([
        request(
          "/v1/ontology/drafts",
          (value): value is DraftResponse[] =>
            Array.isArray(value) && value.every(isDraftResponse),
          signal
        ),
        request(
          "/v1/ontology/revisions",
          (value): value is RevisionResponse[] =>
            Array.isArray(value) && value.every(isRevisionResponse),
          signal
        ),
      ])
      return { drafts, revisions }
    },
    async load(source, signal) {
      const schemaRequest =
        source.kind === "draft"
          ? draft(source.name, signal)
          : source.kind === "revision"
            ? revision(source.id, signal)
            : tagRevision(source.name, signal)
      const [graphResult, schemaResult] = await Promise.all([
        graph(source, signal),
        schemaRequest,
      ])
      return { graph: graphResult, schema: schemaResult.schema }
    },
  }
}
