# Wyse Web Agent Conversation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the static Longzhong conversation preview with a Wyse-native Agent client that creates and resumes conversations, projects retained/live SSE events, and supports send, resume, cancel, and tool approval actions.

**Architecture:** Keep the API contract and UI state separate. `wyse-api.ts` owns JSON HTTP commands, `wyse-event-stream.ts` owns a dependency-free `fetch` SSE reader so HTTP `410 cursor_expired` is observable, and a pure reducer projects `StreamEnvelope` values into stable messages plus temporary LLM/tool/approval UI. `useAgentConversation` coordinates recovery for one selected Agent; `ChatWorkspace` remains a layout shell and consumes only its view model and actions.

**Tech Stack:** React 19, React Router 7, TypeScript, Tailwind CSS 4, Base UI/shadcn, native Fetch/ReadableStream/TextDecoder/AbortController, localStorage, Vitest.

## Global Constraints

- Create an isolated `codex/wyse-web-agent-conversation` worktree before editing implementation files; never work on or commit to `main`.
- Do not add AI SDK, assistant-ui, Zustand, React Query, EventSource, a third-party SSE parser, or any chat/stream runtime dependency.
- The only new dependency is `vitest` as a development dependency; do not add a mock-server, DOM-test, or SSE-test package.
- Preserve the main conversation layout exactly: keep `ChatWorkspace`'s `data-slot="chat-main"`, `flex h-[80dvh] min-h-[36rem] min-w-0 flex-col` class set, its centered main column, `MessageScrollerProvider`, `MessageScroller`, and composer inside that column. Do not wrap the message flow in a new outer card, relocate the composer, alter its height, or add a right-side event rail.
- Keep the existing Hero and Glass navigation untouched. Keep the current history rail visually isolated from the main chat canvas and retain the responsive single-column behavior.
- Server state is authoritative. Persist only recent-Agent entries and transport cursors in localStorage; never cache full messages, tool results, prompts, credentials, or reasoning there.
- Stable messages use `(agent_id, business_seq)` for identity and ordering. Transport cursors are used only in `after_cursor`; never compare or sort them as business sequences.
- Build-time config is `VITE_WYSE_API_BASE_URL` and `VITE_DEFAULT_AGENT_NAME`. Do not hard-code an Agent/template name in React source.
- Do not commit `docs/superpowers/` process documents. Stage only implementation files for implementation commits.

## File Structure

```text
wyse-web/
├── .env.example                                      # Required browser build-time API/template configuration
├── package.json                                      # Adds test script and Vitest dev dependency
├── pnpm-lock.yaml
├── README.md                                         # Local configuration and verification instructions
└── app/
    ├── lib/
    │   ├── wyse-api.ts                               # HTTP DTOs, commands, HTTP/API error mapping
    │   ├── wyse-event-stream.ts                      # fetch SSE framing, cursor capture, abortable subscription
    │   ├── recent-agents.ts                          # Small localStorage recent-Agent/cursor functions
    │   ├── wyse-api.test.ts
    │   ├── wyse-event-stream.test.ts
    │   └── recent-agents.test.ts
    ├── features/agent-conversation/
    │   ├── types.ts                                  # Render-safe state and action types
    │   ├── reducer.ts                                # Pure StreamEnvelope projection
    │   ├── reducer.test.ts
    │   ├── recovery.ts                               # Testable SSE-first fixed-barrier recovery coordinator
    │   └── recovery.test.ts
    ├── hooks/
    │   └── use-agent-conversation.ts                 # React lifecycle, selected Agent and command actions
    ├── components/
    │   ├── chat-workspace.tsx                        # Existing layout shell; replaces static data only
    │   ├── agent-message-list.tsx                    # Stable/draft message, reasoning and tool-process rows
    │   ├── agent-approval-card.tsx                   # Approval details and decision controls
    │   └── chat-workspace.test.tsx                   # Server-rendered main-canvas layout guard
    └── locales/
        ├── en.json
        └── zh.json
```

---

### Task 1: Establish browser configuration and the minimal test command

**Files:**
- Create: `wyse-web/.env.example`
- Modify: `wyse-web/package.json`
- Modify: `wyse-web/pnpm-lock.yaml`
- Modify: `wyse-web/README.md`

**Interfaces:**
- Consumes: Vite's existing `import.meta.env` support.
- Produces: `pnpm --dir wyse-web test` and documented `VITE_WYSE_API_BASE_URL` / `VITE_DEFAULT_AGENT_NAME` configuration for Tasks 2–6.

- [ ] **Step 1: Add a failing test command and prove the current project has no test runner**

Run:

```bash
pnpm --dir wyse-web test
```

Expected: the command fails because the `test` script does not exist.

- [ ] **Step 2: Add Vitest without changing production dependencies**

Run:

```bash
pnpm --dir wyse-web add -D vitest
```

Update `wyse-web/package.json` so its scripts include:

```json
{
  "test": "vitest run --passWithNoTests"
}
```

Do not add `jsdom`, Testing Library, MSW, or an SSE package. Vitest's default Node environment is sufficient for the pure client/reducer tests in this plan.

- [ ] **Step 3: Document the exact local configuration**

Create `wyse-web/.env.example` with:

```dotenv
VITE_WYSE_API_BASE_URL=http://127.0.0.1:8080
VITE_DEFAULT_AGENT_NAME=coding-agent
```

Append to `wyse-web/README.md`:

```markdown
## Agent API development

Copy `.env.example` to `.env.local` and set the API base URL and default
template name. The API origin must appear in `api.allowed_origins` in
`wyse-api` configuration.

```bash
pnpm install
pnpm dev
pnpm typecheck
pnpm test
pnpm build
```
```

- [ ] **Step 4: Verify the test foundation**

Run:

```bash
pnpm --dir wyse-web test
pnpm --dir wyse-web typecheck
```

Expected: Vitest reports zero test files without a configuration failure, and TypeScript passes.

- [ ] **Step 5: Commit only the setup files**

```bash
git add wyse-web/.env.example wyse-web/package.json wyse-web/pnpm-lock.yaml wyse-web/README.md
git commit -m "test(wyse-web): add conversation test foundation"
```

---

### Task 2: Implement the typed HTTP client and an abortable fetch SSE reader

**Files:**
- Create: `wyse-web/app/lib/wyse-api.ts`
- Create: `wyse-web/app/lib/wyse-event-stream.ts`
- Create: `wyse-web/app/lib/wyse-api.test.ts`
- Create: `wyse-web/app/lib/wyse-event-stream.test.ts`

**Interfaces:**
- Consumes: `VITE_WYSE_API_BASE_URL`, browser `fetch`, `ReadableStream`, `TextDecoder`, and API paths from the approved `wyse-api` design.
- Produces: `createWyseApi(options)`, `subscribeToAgentEvents(options)`, `ApiError`, `AgentView`, `HistoryPage`, `StreamEnvelope`, and `SseEvent` for Tasks 4 and 5.

- [ ] **Step 1: Write failing HTTP-client tests**

In `wyse-web/app/lib/wyse-api.test.ts`, define an injected fetch implementation and cover create, send, and structured errors:

```ts
import { describe, expect, it, vi } from "vitest"
import { ApiError, createWyseApi } from "~/lib/wyse-api"

describe("createWyseApi", () => {
  it("posts the first message with the configured template", async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ agent_id: "agent-1", agent_name: "coding-agent", run_id: "run-1" }), {
        status: 201,
        headers: { "content-type": "application/json" },
      })
    )
    const api = createWyseApi({ baseUrl: "https://api.example.test", fetcher })

    await api.createAgent({ agentName: "coding-agent", text: "Inspect the event bus" })

    expect(fetcher).toHaveBeenCalledWith(
      "https://api.example.test/v1/agents",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ agent_name: "coding-agent", text: "Inspect the event bus" }),
      })
    )
  })

  it("surfaces the API's stable error code", async () => {
    const api = createWyseApi({
      baseUrl: "https://api.example.test",
      fetcher: vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ error: { code: "agent_busy", message: "agent is busy" } }), {
          status: 409,
          headers: { "content-type": "application/json" },
        })
      ),
    })

    await expect(api.sendMessage("agent-1", "next")).rejects.toMatchObject<ApiError>({
      code: "agent_busy",
      status: 409,
    })
  })
})
```

- [ ] **Step 2: Run the HTTP test to verify it fails**

Run:

```bash
pnpm --dir wyse-web test -- wyse-api.test.ts
```

Expected: FAIL because `~/lib/wyse-api` does not exist.

- [ ] **Step 3: Define the wire types and minimal JSON command client**

Implement `wyse-api.ts` with the following public surface; retain snake_case only at the wire boundary and convert request fields there:

```ts
export class ApiError extends Error {
  constructor(
    readonly code: string,
    readonly status: number,
    message: string
  ) {
    super(message)
    this.name = "ApiError"
  }
}

export type AgentView = {
  agent_id: string
  agent_name: string
  status: "idle" | "running"
  run_id: string | null
  turn_id: string | null
  last_seq: number
  updated_at: string
}

export type ToolCall = { call_id: string; name: string; arguments: unknown }
export type ChatMessage = {
  role: "user" | "assistant" | "tool" | "system"
  content: { type: "text"; data: string } | { type: "json"; data: unknown }
  tool_calls: readonly ToolCall[]
  reasoning_content?: string
  tool_call_id?: string
}
export type LlmEvent =
  | { type: "text_delta"; data: { role: "assistant" | "user" | "tool" | "system"; delta: string } }
  | { type: "reasoning_delta"; data: { delta: string } }
  | { type: "tool_call_started"; data: { call_id: string; name: string | null } }
  | { type: "tool_call_delta"; data: { call_id: string; name: string | null; arguments_delta: string } }
  | { type: "tool_call_finished"; data: { call_id: string; result: unknown } }
  | { type: "tool_call_failed"; data: { call_id: string; error_text: string } }
  | { type: "started" }
  | { type: "finished"; data: { finish_reason: string; usage: unknown } }
  | { type: "failed"; data: { error_text: string } }
export type AgentEvent =
  | { type: "message"; data: { turn_id: string; message: ChatMessage } }
  | { type: "started"; data: { turn_id: string } }
  | { type: "finished"; data: { finish_reason: string; usage: unknown } }
  | { type: "failed"; data: { error_text: string; usage: unknown } }
  | { type: "cancelled"; data: { usage: unknown } }
  | { type: "tool_approval_requested"; data: { approval_id: string; agent_name: string; call_id: string; tool_name: string; arguments: unknown; tool_kind: "read" | "write"; danger_level: "low" | "medium" | "high" } }
  | { type: "tool_approval_resolved"; data: { approval_id: string; decision: "approve" | "reject" } }
  | { type: "llm"; data: { llm_call_id: string; event: LlmEvent } }

export type StreamEnvelope = {
  business_seq?: number
  run_id: string
  timestamp: string
  event: {
    type: "agent"
    data: { agent_id: string; event: AgentEvent }
  }
}

export type HistoryPage = {
  through_seq: number
  events: readonly StreamEnvelope[]
  next_front_seq: number
  has_more: boolean
}

export type WyseApi = {
  createAgent(input: { agentName: string; text: string }): Promise<{ agent_id: string; agent_name: string; run_id: string }>
  getAgent(agentId: string): Promise<AgentView>
  getHistory(agentId: string, query: { afterSeq: number; throughSeq: number; limit: number }): Promise<HistoryPage>
  sendMessage(agentId: string, text: string): Promise<void>
  resume(agentId: string): Promise<void>
  cancel(agentId: string): Promise<void>
  resolveApproval(agentId: string, approvalId: string, decision: "approve" | "reject"): Promise<void>
}
```

Every JSON command must send `content-type: application/json`. `getHistory` must emit `after_seq`, `through_seq`, and `limit` query parameters. For all non-2xx replies, parse `{ error: { code, message } }` when possible; otherwise throw `new ApiError("http_error", response.status, "request failed")`. Never include a response body in an error message.

- [ ] **Step 4: Write failing SSE-frame tests, including HTTP 410**

In `wyse-web/app/lib/wyse-event-stream.test.ts`, use a `ReadableStream<Uint8Array>` split across arbitrary chunks:

```ts
const streamFrom = (chunks: readonly string[]) =>
  new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(new TextEncoder().encode(chunk))
      controller.close()
    },
  })

it("joins multi-line data and keeps the transport cursor", async () => {
  const seen: SseEvent[] = []
  await readSseStream(
    streamFrom(["id: 41\\nevent: llm\\ndata: {\\\"run_id\\\":\\\"run-1\\\",\\n", "data: \\\"event\\\":{}}\\n\\n"]),
    (event) => seen.push(event)
  )

  expect(seen).toEqual([{ id: "41", event: "llm", data: '{"run_id":"run-1",\n"event":{}}' }])
})

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
```

- [ ] **Step 5: Implement the standards-compatible minimal SSE reader**

Implement these exact exported types and behavior in `wyse-event-stream.ts`:

```ts
export type SseEvent = { id: string | null; event: string; data: string }

export async function readSseStream(
  stream: ReadableStream<Uint8Array>,
  onEvent: (event: SseEvent) => void
): Promise<void>

export function subscribeToAgentEvents(options: {
  baseUrl: string
  agentId: string
  afterCursor?: string
  signal?: AbortSignal
  fetcher?: typeof fetch
  onEvent(event: SseEvent): void
}): { done: Promise<void> }
```

`subscribeToAgentEvents` must issue `fetch(`${baseUrl}/v1/agents/${agentId}/events?...`, { headers: { Accept: "text/event-stream" }, signal })`. Before reading `response.body`, map `410` to `ApiError("cursor_expired", 410, "event cursor expired")`, map other non-2xx status with the same safe error parser as `wyse-api.ts`, and reject an empty body with `ApiError("invalid_stream", 500, "event stream has no body")`.

`readSseStream` must keep an unfinished final line between reader chunks, ignore comment lines beginning with `:`, collect `data:` lines with `\n`, set the current event's `id:` and `event:`, and dispatch only when an empty line terminates an event with data. The hook will parse `event.data` as JSON; the reader must not translate Wyse event types.

- [ ] **Step 6: Run the client and stream tests to verify they pass**

Run:

```bash
pnpm --dir wyse-web test -- wyse-api.test.ts wyse-event-stream.test.ts
pnpm --dir wyse-web typecheck
```

Expected: all targeted tests pass and TypeScript reports no errors.

- [ ] **Step 7: Commit the transport boundary**

```bash
git add wyse-web/app/lib/wyse-api.ts wyse-web/app/lib/wyse-event-stream.ts wyse-web/app/lib/wyse-api.test.ts wyse-web/app/lib/wyse-event-stream.test.ts
git commit -m "feat(wyse-web): add native agent API transport"
```

---

### Task 3: Persist only recent Agent entry points and transport cursors

**Files:**
- Create: `wyse-web/app/lib/recent-agents.ts`
- Create: `wyse-web/app/lib/recent-agents.test.ts`

**Interfaces:**
- Consumes: browser `Storage` and Agent IDs from `WyseApi.createAgent`.
- Produces: `loadRecentAgents`, `rememberRecentAgent`, `removeRecentAgent`, `loadCursor`, `saveCursor`, and `clearCursor` for Task 5.

- [ ] **Step 1: Write failing persistence tests with an in-memory Storage**

```ts
import { describe, expect, it } from "vitest"
import { createMemoryStorage, loadRecentAgents, rememberRecentAgent } from "~/lib/recent-agents"

it("moves a reopened Agent to the front without duplicating it", () => {
  const storage = createMemoryStorage()
  rememberRecentAgent(storage, { agentId: "agent-1", agentName: "coding-agent", title: "First request", lastOpenedAt: "2026-07-11T00:00:00Z" })
  rememberRecentAgent(storage, { agentId: "agent-2", agentName: "coding-agent", title: "Second request", lastOpenedAt: "2026-07-11T00:01:00Z" })
  rememberRecentAgent(storage, { agentId: "agent-1", agentName: "coding-agent", title: "Reopened", lastOpenedAt: "2026-07-11T00:02:00Z" })

  expect(loadRecentAgents(storage).map((agent) => agent.agentId)).toEqual(["agent-1", "agent-2"])
})
```

- [ ] **Step 2: Run the persistence test to verify it fails**

Run:

```bash
pnpm --dir wyse-web test -- recent-agents.test.ts
```

Expected: FAIL because `~/lib/recent-agents` does not exist.

- [ ] **Step 3: Implement bounded, safe localStorage helpers**

Use keys `wyse-recent-agents` and `wyse-agent-cursor:<agentId>`. Define:

```ts
export type RecentAgent = {
  agentId: string
  agentName: string
  title: string
  lastOpenedAt: string
}

export type StorageLike = Pick<Storage, "getItem" | "setItem" | "removeItem">

export function loadRecentAgents(storage: StorageLike): RecentAgent[]
export function rememberRecentAgent(storage: StorageLike, agent: RecentAgent): void
export function removeRecentAgent(storage: StorageLike, agentId: string): void
export function loadCursor(storage: StorageLike, agentId: string): string | undefined
export function saveCursor(storage: StorageLike, agentId: string, cursor: string): void
export function clearCursor(storage: StorageLike, agentId: string): void
```

Malformed JSON must return an empty recent list and remove the malformed value. `rememberRecentAgent` must deduplicate by `agentId`, place the updated entry first, and keep only 20 entries. Cursor helpers must only read/write the cursor key and never serialize messages or events.

- [ ] **Step 4: Run the persistence tests to verify they pass**

Run:

```bash
pnpm --dir wyse-web test -- recent-agents.test.ts
```

Expected: tests pass for deduplication, malformed JSON recovery, removal, and cursor isolation.

- [ ] **Step 5: Commit the local entry-point store**

```bash
git add wyse-web/app/lib/recent-agents.ts wyse-web/app/lib/recent-agents.test.ts
git commit -m "feat(wyse-web): remember local agent entry points"
```

---

### Task 4: Build the pure Wyse event projector and reducer

**Files:**
- Create: `wyse-web/app/features/agent-conversation/types.ts`
- Create: `wyse-web/app/features/agent-conversation/reducer.ts`
- Create: `wyse-web/app/features/agent-conversation/reducer.test.ts`

**Interfaces:**
- Consumes: `AgentView`, `StreamEnvelope`, and `AgentEvent` from `~/lib/wyse-api`.
- Produces: `ConversationState`, `ConversationAction`, `initialConversationState`, and `conversationReducer` for Tasks 5–6.

- [ ] **Step 1: Write failing reducer tests for the protocol invariants**

```ts
const messageEnvelope = (agentId: string, businessSeq: number, text: string): StreamEnvelope => ({
  business_seq: businessSeq,
  run_id: "run-1",
  timestamp: "2026-07-11T00:00:00Z",
  event: {
    type: "agent",
    data: {
      agent_id: agentId,
      event: {
        type: "message",
        data: {
          turn_id: "turn-1",
          message: { role: "assistant", content: { type: "text", data: text }, tool_calls: [] },
        },
      },
    },
  },
})

const llmEnvelope = (agentId: string, llmCallId: string, event: LlmEvent): StreamEnvelope => ({
  business_seq: undefined,
  run_id: "run-1",
  timestamp: "2026-07-11T00:00:00Z",
  event: { type: "agent", data: { agent_id: agentId, event: { type: "llm", data: { llm_call_id: llmCallId, event } } } },
})

const reduceAll = (state: ConversationState, actions: readonly ConversationAction[]) =>
  actions.reduce(conversationReducer, state)

it("keeps one stable message when history and SSE replay share a business sequence", () => {
  const state = reduceAll(initialConversationState, [
    { type: "history_loaded", events: [messageEnvelope("agent-1", 7, "history")] },
    { type: "envelope_received", envelope: messageEnvelope("agent-1", 7, "replay") },
  ])

  expect(state.messages).toHaveLength(1)
  expect(state.messages[0]).toMatchObject({ businessSeq: 7, text: "history" })
})

it("accumulates text and reasoning by LLM call without changing stable history", () => {
  const state = reduceAll(initialConversationState, [
    { type: "envelope_received", envelope: llmEnvelope("agent-1", "llm-1", { type: "text_delta", data: { role: "assistant", delta: "hel" } }) },
    { type: "envelope_received", envelope: llmEnvelope("agent-1", "llm-1", { type: "reasoning_delta", data: { delta: "plan" } }) },
  ])

  expect(state.messages).toHaveLength(0)
  expect(state.drafts["llm-1"]).toMatchObject({ text: "hel", reasoning: "plan" })
})
```

Also add tests for ascending `business_seq` sort, `ToolCallStarted`/`ToolCallDelta`/`ToolCallFinished`, approval requested/resolved, `Finished` clearing drafts, and an envelope whose `agent_id` differs from the selected Agent being ignored.

- [ ] **Step 2: Run the reducer tests to verify they fail**

Run:

```bash
pnpm --dir wyse-web test -- reducer.test.ts
```

Expected: FAIL because the feature module does not exist.

- [ ] **Step 3: Implement render-safe conversation state and projection**

In `types.ts`, define view-oriented records rather than copying the complete server state:

```ts
export type StableMessage = {
  agentId: string
  businessSeq: number
  role: "user" | "assistant" | "tool" | "system"
  text: string | null
  json: unknown | null
  reasoning: string | null
  toolCalls: readonly { callId: string; name: string; arguments: unknown }[]
  timestamp: string
}

export type ConversationState = {
  agentId: string | null
  view: AgentView | null
  messages: readonly StableMessage[]
  drafts: Readonly<Record<string, { text: string; reasoning: string }>>
  tools: Readonly<Record<string, ToolProgress>>
  approvals: Readonly<Record<string, ApprovalRequest>>
  phase: "empty" | "recovering" | "ready" | "connection_error" | "missing"
  error: ApiError | null
}
```

In `reducer.ts`, key stable messages internally by `${agentId}:${businessSeq}`, replace only on the first arrival, and derive `messages` in ascending `businessSeq` order. Project only `RuntimeEvent::Agent` payloads. `LlmEvent::TextDelta` contributes text only for `role: "assistant"`; reasoning and tool state are transient. `AgentEvent::Finished`, `Failed`, and `Cancelled` update `view.status` and clear transient drafts; approval resolution removes only the matching approval.

- [ ] **Step 4: Run reducer tests and typecheck**

Run:

```bash
pnpm --dir wyse-web test -- reducer.test.ts
pnpm --dir wyse-web typecheck
```

Expected: all projector tests pass and the feature types do not leak `any`.

- [ ] **Step 5: Commit the pure projection layer**

```bash
git add wyse-web/app/features/agent-conversation/types.ts wyse-web/app/features/agent-conversation/reducer.ts wyse-web/app/features/agent-conversation/reducer.test.ts
git commit -m "feat(wyse-web): project native agent events"
```

---

### Task 5: Implement SSE-first fixed-barrier recovery and React command lifecycle

**Files:**
- Create: `wyse-web/app/features/agent-conversation/recovery.ts`
- Create: `wyse-web/app/features/agent-conversation/recovery.test.ts`
- Create: `wyse-web/app/hooks/use-agent-conversation.ts`

**Interfaces:**
- Consumes: `WyseApi`, `subscribeToAgentEvents`, cursor helpers, and `conversationReducer` from Tasks 2–4.
- Produces: `recoverConversation`, `useAgentConversation`, and the `AgentConversation` view/action interface for Task 6.

- [ ] **Step 1: Write failing recovery tests for no-gap ordering and expiry**

```ts
const agentView = (overrides: Partial<AgentView> = {}): AgentView => ({
  agent_id: "agent-1",
  agent_name: "coding-agent",
  status: "idle",
  run_id: null,
  turn_id: null,
  last_seq: 0,
  updated_at: "2026-07-11T00:00:00Z",
  ...overrides,
})

const messageEnvelope = (agentId: string, businessSeq: number, text: string): StreamEnvelope => ({
  business_seq: businessSeq,
  run_id: "run-1",
  timestamp: "2026-07-11T00:00:00Z",
  event: { type: "agent", data: { agent_id: agentId, event: { type: "message", data: { turn_id: "turn-1", message: { role: "assistant", content: { type: "text", data: text }, tool_calls: [] } } } } },
})

const createRecoveryHarness = (overrides: Partial<RecoveryDependencies>) => {
  const controller = new AbortController()
  const dependencies: RecoveryDependencies = {
    api: { getAgent: async () => agentView(), getHistory: async () => historyPage([], 0, false) },
    subscribe: () => ({ done: new Promise(() => {}) }),
    loadCursor: () => undefined,
    saveCursor: () => {},
    clearCursor: () => {},
    dispatch: () => {},
    ...overrides,
  }
  return { recover: (agentId: string) => recoverConversation(dependencies, { agentId, signal: controller.signal }) }
}

it("starts the stream before reading AgentView and drains buffered events after fixed history", async () => {
  const calls: string[] = []
  const dispatched: ConversationAction[] = []
  const recovery = createRecoveryHarness({
    subscribe: ({ onEvent }) => {
      calls.push("subscribe")
      onEvent(sseEnvelope("11", messageEnvelope("agent-1", 3, "live")))
      return { done: new Promise(() => {}) }
    },
    api: {
      getAgent: async () => { calls.push("view"); return agentView({ last_seq: 2 }) },
      getHistory: async () => { calls.push("history"); return historyPage([messageEnvelope("agent-1", 2, "stored")], 2, false) },
    },
    dispatch: (action) => dispatched.push(action),
  })

  await recovery.recover("agent-1")

  expect(calls).toEqual(["subscribe", "view", "history"])
  expect(dispatched.map((action) => action.type)).toEqual(["recovery_started", "view_loaded", "history_loaded", "envelope_received", "recovery_ready"])
})

it("clears only the expired Agent cursor then restarts with replay=all", async () => {
  const afterCursors: Array<string | undefined> = []
  const clearCursor = vi.fn()
  let attempts = 0
  const recovery = createRecoveryHarness({
    subscribe: ({ afterCursor }: { afterCursor?: string }) => {
      afterCursors.push(afterCursor)
      attempts += 1
      return {
        done: attempts === 1
          ? Promise.reject(new ApiError("cursor_expired", 410, "event cursor expired"))
          : new Promise(() => {}),
      }
    },
    loadCursor: () => "99",
    clearCursor,
    api: { getAgent: async () => agentView({ last_seq: 0 }), getHistory: async () => historyPage([], 0, false) },
    dispatch: vi.fn(),
  })

  await recovery.recover("agent-1")

  expect(clearCursor).toHaveBeenCalledWith("agent-1")
  expect(afterCursors).toEqual(["99", undefined])
})
```

Define the test-local fixture helpers above these cases so each expected protocol shape is explicit:

```ts
const sseEnvelope = (id: string, envelope: StreamEnvelope): SseEvent => ({
  id,
  event: "message",
  data: JSON.stringify(envelope),
})

const historyPage = (events: readonly StreamEnvelope[], throughSeq: number, hasMore: boolean): HistoryPage => ({
  events: [...events], through_seq: throughSeq, next_front_seq: throughSeq, has_more: hasMore,
})
```

- [ ] **Step 2: Run recovery tests to verify they fail**

Run:

```bash
pnpm --dir wyse-web test -- recovery.test.ts
```

Expected: FAIL because the recovery coordinator does not exist.

- [ ] **Step 3: Implement the testable recovery coordinator**

Implement this interface in `recovery.ts`:

```ts
export type RecoveryDependencies = {
  api: Pick<WyseApi, "getAgent" | "getHistory">
  subscribe: typeof subscribeToAgentEvents
  loadCursor(agentId: string): string | undefined
  saveCursor(agentId: string, cursor: string): void
  clearCursor(agentId: string): void
  dispatch(action: ConversationAction): void
}

export async function recoverConversation(
  dependencies: RecoveryDependencies,
  input: { agentId: string; signal: AbortSignal }
): Promise<void>
```

Call `subscribe` first and push parsed envelopes into a local buffer while `getAgent` and fixed-range `getHistory` requests run. Dispatch `history_loaded` pages before buffered `envelope_received` actions. Persist a cursor only after its envelope has been accepted for the selected Agent. On `ApiError` code `cursor_expired`, clear that Agent's cursor and rerun once with no cursor; any second error becomes `connection_error`. Never retry command POSTs automatically.

- [ ] **Step 4: Implement the hook around one abortable selected Agent**

`useAgentConversation.ts` must use `useReducer(conversationReducer, initialConversationState)` and one effect keyed by selected `agentId`. Each effect creates an `AbortController`, calls `recoverConversation`, and aborts in cleanup. Guard every late completion with a monotonically increasing selection generation so a previous Agent cannot dispatch into a new selection.

Expose exactly:

```ts
export type AgentConversation = {
  state: ConversationState
  recentAgents: readonly RecentAgent[]
  selectAgent(agentId: string | null): void
  createConversation(text: string): Promise<void>
  sendMessage(text: string): Promise<void>
  resume(): Promise<void>
  cancel(): Promise<void>
  resolveApproval(approvalId: string, decision: "approve" | "reject"): Promise<void>
  reconnect(): void
  removeRecentAgent(agentId: string): void
}
```

`createConversation` must validate `text.trim()`, fail visibly if either required Vite variable is absent, call `api.createAgent`, store `{ agentId, agentName, title: text.trim(), lastOpenedAt: new Date().toISOString() }`, then select the returned Agent. `sendMessage` clears the composer only after a `202` succeeds; it must not add an optimistic stable message. `resume_not_running` refetches/reconnects without a terminal error; `cancel` transforms `resume_required` into a recoverable state; approval `204` leaves the card until its resolved event arrives.

- [ ] **Step 5: Run recovery tests and all pure tests**

Run:

```bash
pnpm --dir wyse-web test -- recovery.test.ts reducer.test.ts wyse-api.test.ts wyse-event-stream.test.ts recent-agents.test.ts
pnpm --dir wyse-web typecheck
```

Expected: buffered live messages are not lost, `410` is detected through `fetch`, and stale Agent work is ignored.

- [ ] **Step 6: Commit the lifecycle layer**

```bash
git add wyse-web/app/features/agent-conversation/recovery.ts wyse-web/app/features/agent-conversation/recovery.test.ts wyse-web/app/hooks/use-agent-conversation.ts
git commit -m "feat(wyse-web): recover one native agent conversation"
```

---

### Task 6: Replace the static preview without changing the main chat canvas

**Files:**
- Create: `wyse-web/app/components/agent-message-list.tsx`
- Create: `wyse-web/app/components/agent-approval-card.tsx`
- Create: `wyse-web/app/components/chat-workspace.test.tsx`
- Modify: `wyse-web/app/components/chat-workspace.tsx`
- Modify: `wyse-web/app/locales/en.json`
- Modify: `wyse-web/app/locales/zh.json`

**Interfaces:**
- Consumes: `AgentConversation` from `useAgentConversation` and view records from `types.ts`.
- Produces: the live Agent UI while preserving the existing chat canvas's structural contract.

- [ ] **Step 1: Write the failing server-rendered layout guard**

Use `react-dom/server` and a Vitest mock of `useAgentConversation`; do not add a DOM test package:

```tsx
import { renderToStaticMarkup } from "react-dom/server"
import { describe, expect, it, vi } from "vitest"

vi.mock("~/hooks/use-agent-conversation", () => ({
  useAgentConversation: () => ({ state: readyState, recentAgents: [], selectAgent: vi.fn(), createConversation: vi.fn(), sendMessage: vi.fn(), resume: vi.fn(), cancel: vi.fn(), resolveApproval: vi.fn(), reconnect: vi.fn(), removeRecentAgent: vi.fn() }),
}))

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

const readyState = {
  agentId: null,
  view: null,
  messages: [],
  drafts: {},
  tools: {},
  approvals: {},
  phase: "empty" as const,
  error: null,
}

it("keeps the established main conversation canvas and composer placement", async () => {
  const { ChatWorkspace } = await import("~/components/chat-workspace")
  const html = renderToStaticMarkup(<ChatWorkspace />)

  expect(html).toContain('data-slot="chat-main"')
  expect(html).toContain("h-[80dvh]")
  expect(html).toContain("min-h-[36rem]")
  expect(html.indexOf('data-slot="message-scroller"')).toBeLessThan(html.indexOf('data-slot="card"'))
})
```

- [ ] **Step 2: Run the layout guard to verify it fails**

Run:

```bash
pnpm --dir wyse-web test -- chat-workspace.test.tsx
```

Expected: FAIL because the hook and live component contract do not exist.

- [ ] **Step 3: Implement focused presentation components**

`agent-message-list.tsx` must render existing `Message`, `Bubble`, and `MessageScrollerItem` primitives. Stable user messages remain end-aligned secondary bubbles; stable assistant messages remain start-aligned ghost rows. Render drafts after stable messages with a visible streaming marker, reasoning in a native `<details>` element, and tool process details in a second `<details>` element. Render text as plain text in this phase; do not add Markdown/code dependencies.

`agent-approval-card.tsx` must receive:

```tsx
type AgentApprovalCardProps = {
  approval: ApprovalRequest
  submitting: boolean
  onDecision(decision: "approve" | "reject"): void
}
```

Display the tool name, `toolKind`, `dangerLevel`, and `JSON.stringify(approval.arguments, null, 2)` in a scrollable `<pre>`. Use the existing `Button` for an approve action and `variant="destructive"` for reject. Disable both only while that approval is submitting; do not remove the card locally.

- [ ] **Step 4: Wire the existing `ChatWorkspace` shell to live state**

Replace the `historyItems` and `messages` fixtures with `const conversation = useAgentConversation()`. Keep the current section, parent wrapper, history `Card`, and this exact main node unchanged:

```tsx
<div
  data-slot="chat-main"
  className="flex h-[80dvh] min-h-[36rem] min-w-0 flex-col"
>
```

Keep `MessageScrollerProvider`, `MessageScroller`, `MessageScrollerViewport`, `MessageScrollerContent`, and `MessageScrollerButton` in their current nesting order. Insert `AgentMessageList` inside the existing `MessageScrollerContent`; do not add a parent card around it. Keep the composer `Card` as the final child of `chat-main`, use a controlled `Textarea`, and submit on the existing send button.

The history card renders `conversation.recentAgents`, selects an Agent on click, and allows removing a `missing` entry. Its New button calls `conversation.selectAgent(null)` and focuses the composer; the next nonblank submit calls `createConversation` because there is no selected Agent. When an Agent is selected, the same submit calls `sendMessage`. The composer footer renders the current connection/error text plus exactly one contextual action: reconnect, continue, or cancel. It must not reserve a third column or modify the Hero.

- [ ] **Step 5: Replace static-preview copy with complete live-state copy**

Add equivalent English and Chinese translation keys for: empty conversation, start/new conversation, creating/sending, connecting/reconnecting, connection failed, resume/continue, cancel, missing conversation, remove local entry, approval request, approve/reject, tool kind, danger level, and stream status. Remove `chat.history.localOnly` and `chat.composer.hint` copy that claims this is a static preview.

- [ ] **Step 6: Run the UI guard, typecheck, and production build**

Run:

```bash
pnpm --dir wyse-web test -- chat-workspace.test.tsx
pnpm --dir wyse-web typecheck
pnpm --dir wyse-web build
```

Expected: the layout guard passes, TypeScript passes, and the React Router production build completes.

- [ ] **Step 7: Commit the live presentation**

```bash
git add wyse-web/app/components/agent-message-list.tsx wyse-web/app/components/agent-approval-card.tsx wyse-web/app/components/chat-workspace.tsx wyse-web/app/components/chat-workspace.test.tsx wyse-web/app/locales/en.json wyse-web/app/locales/zh.json
git commit -m "feat(wyse-web): connect the agent chat workspace"
```

---

### Task 7: Run the full verification gate and review the final client boundary

**Files:**
- Modify only if verification exposes a defect: the exact file owning that defect.

**Interfaces:**
- Consumes: all completed production and test files from Tasks 1–6.
- Produces: verified production build with no forbidden chat-runtime dependency and an unchanged main chat canvas.

- [ ] **Step 1: Run the complete Web verification suite**

Run:

```bash
pnpm --dir wyse-web typecheck
pnpm --dir wyse-web test
pnpm --dir wyse-web build
```

Expected: all commands exit with status 0.

- [ ] **Step 2: Audit dependencies and layout invariants**

Run:

```bash
rg -n '"(ai|@ai-sdk|assistant-ui|zustand|@tanstack/react-query|eventsource)"' wyse-web/package.json wyse-web/pnpm-lock.yaml
rg -n 'data-slot="chat-main"|h-\[80dvh\]|min-h-\[36rem\]|MessageScrollerProvider|MessageScrollerViewport' wyse-web/app/components/chat-workspace.tsx
```

Expected: the dependency scan returns no forbidden dependency entry; the layout scan returns the unchanged main chat slot, height/min-height, and scroller nesting.

- [ ] **Step 3: Perform manual browser acceptance against a running `wyse-api`**

Verify these exact user-visible flows:

1. Create a default-template Agent with the first message; its local history entry appears and the first persisted message arrives through SSE/history.
2. Refresh during output; history has no duplicate stable message and the next connection uses the saved cursor.
3. Force a stale cursor; the client receives HTTP `410`, clears only that Agent cursor, and restores the full history.
4. Send an additional message; no unsequenced optimistic message appears before server events arrive.
5. Trigger a tool approval; approve and reject each keep the card until `ToolApprovalResolved`.
6. Switch between two local Agent entries while one stream is active; no late event from the previous Agent appears in the selected conversation.
7. Resize below `768px`; the history summary remains above the unchanged main conversation canvas and no horizontal overflow or right rail appears.

- [ ] **Step 4: Commit any verification-only fixes, if and only if files changed**

```bash
if [ -n "$(git status --short wyse-web)" ]; then
  git add wyse-web
  git commit -m "fix(wyse-web): resolve conversation verification findings"
fi
```

The command stages only the isolated worktree's Web changes. If its status is empty, it creates no commit.

## Plan Self-Review

- Spec coverage: Task 1 covers build configuration; Task 2 covers HTTP and observable SSE status; Task 3 covers only local entry/cursor persistence; Task 4 covers native event projection; Task 5 covers the required SSE-first/fixed-history recovery and commands; Task 6 covers the main layout-preserving UI and localization; Task 7 covers typecheck, tests, build, forbidden dependencies, and browser behavior.
- Scope: no task adds a server list endpoint, template discovery UI, AI SDK, assistant-ui, Markdown, attachments, message editing, regeneration, branches, a right event rail, or a global client store.
- Type consistency: `WyseApi` and `subscribeToAgentEvents` originate in Task 2; `ConversationState` and `ConversationAction` originate in Task 4; `recoverConversation` consumes them in Task 5; only Task 6 consumes the exported `AgentConversation` hook shape.
- Placeholder scan: this plan contains no deferred implementation markers; the sole conditional verification commit is intentionally skipped when no files change.
