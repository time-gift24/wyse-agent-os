import { describe, expect, it, vi } from "vitest"

import { ApiError, createWyseApi } from "~/lib/wyse-api"

describe("createWyseApi", () => {
  it("posts the first message with the configured template", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          agent_id: "agent-1",
          agent_name: "coding-agent",
          run_id: "run-1",
        }),
        {
          status: 201,
          headers: { "content-type": "application/json" },
        }
      )
    )
    const api = createWyseApi({ baseUrl: "https://api.example.test", fetcher })

    await api.createAgent({
      agentName: "coding-agent",
      text: "Inspect the event bus",
    })

    expect(fetcher).toHaveBeenCalledWith(
      "https://api.example.test/v1/agents",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          agent_name: "coding-agent",
          text: "Inspect the event bus",
        }),
      })
    )
  })

  it("surfaces the API's stable error code", async () => {
    const api = createWyseApi({
      baseUrl: "https://api.example.test",
      fetcher: vi.fn().mockResolvedValue(
        new Response(
          JSON.stringify({
            error: { code: "agent_busy", message: "agent is busy" },
          }),
          {
            status: 409,
            headers: { "content-type": "application/json" },
          }
        )
      ),
    })

    await expect(api.sendMessage("agent-1", "next")).rejects.toMatchObject({
      code: "agent_busy",
      status: 409,
    })
  })
})
