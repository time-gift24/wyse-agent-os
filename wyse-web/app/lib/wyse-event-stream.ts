import { ApiError, apiErrorFromResponse } from "~/lib/wyse-api"

export type SseEvent = { id: string | null; event: string; data: string }

export async function readSseStream(
  stream: ReadableStream<Uint8Array>,
  onEvent: (event: SseEvent) => void
): Promise<void> {
  const decoder = new TextDecoder()
  const reader = stream.getReader()
  let buffer = ""
  let id: string | null = null
  let event = "message"
  let data: string[] = []

  const dispatch = () => {
    if (data.length > 0) onEvent({ id, event, data: data.join("\n") })
    id = null
    event = "message"
    data = []
  }

  const readLine = (line: string) => {
    if (line === "") {
      dispatch()
      return
    }
    if (line.startsWith(":")) return

    const separator = line.indexOf(":")
    const field = separator === -1 ? line : line.slice(0, separator)
    const value =
      separator === -1 ? "" : line.slice(separator + 1).replace(/^ /, "")

    if (field === "data") data.push(value)
    else if (field === "event") event = value || "message"
    else if (field === "id" && !value.includes("\0")) id = value
  }

  const consumeLines = (final = false) => {
    let start = 0
    for (let index = 0; index < buffer.length; index += 1) {
      if (buffer[index] === "\r") {
        if (index + 1 === buffer.length && !final) break
        readLine(buffer.slice(start, index))
        if (buffer[index + 1] === "\n") index += 1
        start = index + 1
      } else if (buffer[index] === "\n") {
        readLine(buffer.slice(start, index))
        start = index + 1
      }
    }
    buffer = buffer.slice(start)
  }

  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      buffer += decoder.decode(value, { stream: true })
      consumeLines()
    }
    buffer += decoder.decode()
    consumeLines(true)
  } finally {
    reader.releaseLock()
  }
}

export function subscribeToAgentEvents(options: {
  baseUrl: string
  agentId: string
  afterCursor?: string
  signal?: AbortSignal
  fetcher?: typeof fetch
  onEvent(event: SseEvent): void
}): { done: Promise<void> } {
  const search = new URLSearchParams(
    options.afterCursor
      ? { after_cursor: options.afterCursor }
      : { replay: "all" }
  )
  const url = `${options.baseUrl.replace(/\/$/, "")}/v1/agents/${options.agentId}/events?${search}`
  const fetcher = options.fetcher ?? fetch

  return {
    done: (async () => {
      const response = await fetcher(url, {
        headers: { Accept: "text/event-stream" },
        signal: options.signal,
      })
      if (response.status === 410) {
        throw new ApiError("cursor_expired", 410, "event cursor expired")
      }
      if (!response.ok) throw await apiErrorFromResponse(response)
      if (!response.body) {
        throw new ApiError("invalid_stream", 500, "event stream has no body")
      }
      await readSseStream(response.body, options.onEvent)
    })(),
  }
}
