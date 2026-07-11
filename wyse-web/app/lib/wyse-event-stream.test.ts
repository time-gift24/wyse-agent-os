import { describe, expect, it, vi } from "vitest"

import {
  readSseStream,
  subscribeToAgentEvents,
  type SseEvent,
} from "~/lib/wyse-event-stream"

const streamFrom = (chunks: readonly string[]) =>
  new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks)
        controller.enqueue(new TextEncoder().encode(chunk))
      controller.close()
    },
  })

describe("readSseStream", () => {
  it("joins multi-line data and keeps the transport cursor", async () => {
    const seen: SseEvent[] = []
    await readSseStream(
      streamFrom([
        'id: 41\nevent: llm\ndata: {"run_id":"run-1",\n',
        'data: "event":{}}\n\n',
      ]),
      (event) => seen.push(event)
    )

    expect(seen).toEqual([
      { id: "41", event: "llm", data: '{"run_id":"run-1",\n"event":{}}' },
    ])
  })

  it("ignores comments and dispatches events with data", async () => {
    const seen: SseEvent[] = []
    await readSseStream(
      streamFrom([": keepalive\nid: 42\nevent: agent\ndata: ready\n\n"]),
      (event) => seen.push(event)
    )

    expect(seen).toEqual([{ id: "42", event: "agent", data: "ready" }])
  })

  it("accepts carriage-return line endings split between chunks", async () => {
    const seen: SseEvent[] = []
    await readSseStream(
      streamFrom(["id: 43\r", "\nevent: agent\rdata: ready\r\r"]),
      (event) => seen.push(event)
    )

    expect(seen).toEqual([{ id: "43", event: "agent", data: "ready" }])
  })
})

describe("subscribeToAgentEvents", () => {
  it("reports cursor expiry before reading the stream", async () => {
    await expect(
      subscribeToAgentEvents({
        baseUrl: "https://api.example.test",
        agentId: "agent-1",
        afterCursor: "99",
        fetcher: vi.fn().mockResolvedValue(new Response(null, { status: 410 })),
        onEvent: vi.fn(),
      }).done
    ).rejects.toMatchObject({ code: "cursor_expired", status: 410 })
  })
})
