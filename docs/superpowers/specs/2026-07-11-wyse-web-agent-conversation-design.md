# Wyse Web Agent 对话设计

## 状态

已批准，待编写实施计划。

## 目标

把现有 `wyse-web` 的静态 Longzhong Chat 工作区接入已批准的 `wyse-api` Agent 对话 API，支持：

- 按默认模板创建带首条消息的 Agent；
- 打开本机最近使用过的 Agent；
- 恢复完整消息历史并持续消费 Wyse 原生 SSE；
- 发送后续消息、恢复未完成运行、取消当前运行和处理工具审批；
- 在页面刷新、SSE 断线与 cursor 过期后保持正确恢复。

本阶段不做服务端 Agent 列表、模板发现、消息编辑/重新生成/分支、附件、语音、用户认证或右侧事件面板。

## 已确认的技术选择

- 不引入 AI SDK，包含其 `useChat`、UI 类型、Data Stream 协议和 transport。
- 不引入 assistant-ui、Zustand、React Query 或其他聊天 runtime / 外部状态库。
- 前端直接解释 `wyse-api` 的 HTTP DTO、`HistoryPage` 和 SSE `StreamEnvelope`；后端保持 Wyse 原生协议。
- AI Elements 不作为 npm 依赖或运行时基础。未来只允许按需拷贝某个视觉组件源码，并先改造成接收本设计定义的 Wyse ViewModel；不得带入 AI SDK 类型、hooks 或 transport。
- 保留当前 React Router、React、Base UI/shadcn、现有 `MessageScroller` 和 `ChatWorkspace` 的布局约束。
- 只增加 `vitest` 作为测试开发依赖；不增加 mock server 框架。

## 配置与本地入口

Web 构建时读取以下 Vite 环境变量：

- `VITE_WYSE_API_BASE_URL`：`wyse-api` 的绝对 base URL。部署时必须与 API 的 CORS `allowed_origins` 一致。
- `VITE_DEFAULT_AGENT_NAME`：首版唯一可创建的模板名。

点击“新建对话”后，界面直接用 `VITE_DEFAULT_AGENT_NAME` 和首条非空文本调用 `POST /v1/agents`。首版不展示模板选择器，避免在没有模板列表 API 时把模板名写死进 React 组件。

浏览器使用 localStorage 保存最近访问入口，不保存消息或运行事实：

```ts
type RecentAgent = {
  agentId: string
  agentName: string
  title: string
  lastOpenedAt: string
}
```

列表只包含当前浏览器创建或打开过的 Agent。它不是服务端列表：换浏览器、无痕窗口或清除本地存储后，旧 Agent 不会自动被发现。服务端返回 `404` 时，该入口显示失效状态，并由用户选择移除。

## 模块边界

```text
wyse-web/app/
├── lib/
│   ├── wyse-api.ts                 # HTTP DTO 与类型化 API 错误
│   ├── wyse-event-stream.ts        # fetch SSE 请求、解码和取消
│   └── recent-agents.ts            # localStorage 入口，无消息缓存
├── hooks/
│   └── use-agent-conversation.ts   # 生命周期、恢复编排与 actions
├── features/agent-conversation/
│   ├── reducer.ts                  # 纯状态投影
│   ├── types.ts                    # UI state / view model
│   └── recovery.ts                 # SSE 先连接、固定历史边界与缓冲排空
└── components/
    └── chat-workspace.tsx          # 保持布局，仅消费 ViewModel / actions
```

`wyse-api.ts` 是唯一能知道 URL、HTTP body 和 API error code 的模块。`wyse-event-stream.ts` 用原生 `fetch` 建立 SSE 请求、检查 HTTP status 后增量解码 SSE fields，并由 `AbortController` 取消连接。这样客户端能识别 HTTP `410 cursor_expired`；不引入 EventSource、SSE parser 或其他第三方依赖。刷新恢复继续使用 `after_cursor` 查询参数。

`useAgentConversation` 只管理当前选中的一个 Agent。切换 Agent 或卸载时，它 abort 旧 SSE request、放弃旧恢复任务并忽略旧请求的晚到结果。它不持久化完整消息、不实现通用 store，也不在组件外建立全局聊天 runtime。

`reducer.ts` 没有浏览器 I/O，只把 API 响应和原生事件投影为可渲染状态。`recovery.ts` 以可注入的 API、SSE subscribe、cursor 和 dispatch 函数实现 SSE 先连接、固定边界分页与缓冲排空，因而两者都可独立测试。

## 状态模型

状态分为服务端事实的本地投影与短暂 UI 过程，不能互相替代：

- 稳定消息：由 `HistoryPage.events` 和 `AgentEvent::Message` 产生，键为 `(agent_id, business_seq)`，按 `business_seq` 渲染。相同键只保留一条。
- Agent view：保存 `status`、`run_id`、`turn_id`、usage、`last_seq` 和最近一次更新时间。
- 过程投影：以 `llm_call_id` 累积运行中的可见文本和 reasoning，以 `call_id` 累积工具名、参数片段、状态、结果或错误。
- 待审批：以 `approval_id` 保存 `ToolApprovalRequested` 的工具名、arguments、`tool_kind` 和 `danger_level`；仅 `ToolApprovalResolved` 或重新恢复后的事实状态能将其关闭。
- 传输 cursor：每个 Agent 独立保存最近已接受 SSE event 的 cursor，用于下一次 `after_cursor` 恢复；它绝不参与消息排序。
- UI 状态：初始加载、恢复中、SSE 连接错误、当前提交动作和表单草稿。

完整 `ChatMessage` 的 user、assistant 与 tool 角色都保留在稳定消息中。reasoning 与 tool 过程先以临时视图显示；对应的完整 assistant / tool message 到达时，稳定历史成为最终渲染事实。

## 恢复与实时事件

打开或创建 Agent 时严格执行后端设计规定的顺序：

1. 用 `fetch` 建立 SSE。首次使用 `GET /events?replay=all`；已有该 Agent cursor 时使用 `GET /events?after_cursor=<cursor>`。
2. SSE 在恢复期间把 retained 和 live event 按到达顺序放入内存缓冲，同时记录已接受 cursor。
3. 读取 `GET /v1/agents/{agent_id}` 获得 `last_seq` 与状态。
4. 以该 `last_seq` 作为固定 `through_seq`，重复请求 `/messages` 直到 `has_more=false`。
5. 先投影固定历史，再按原顺序排空 SSE 缓冲；稳定消息根据 `(agent_id, business_seq)` 去重。之后该 SSE request 的事件直接进入 reducer。

最小解码器按 SSE 标准增量处理 `id:`、`event:`、多行 `data:`、空行 dispatch 和 keep-alive comment；它使用服务端提供的 `id` 作为 transport cursor，并把完整 JSON data 解析为 `StreamEnvelope`。client 接受已知命名 event：`message`、`started`、`finished`、`failed`、`cancelled`、`tool_approval_requested`、`tool_approval_resolved`、`llm` 与 `stream_error`。未知 event 安全忽略并保留诊断信息，不扩展服务端协议。

收到 HTTP `410 cursor_expired` 时，清除该 Agent cursor、abort 旧流并执行完整恢复。收到流内 `stream_error`、fetch/network error 或非 `2xx` 响应时，保留当前已投影内容，标记连接错误并提供用户触发的重新连接；重新连接优先使用最近 cursor。

## 用户操作

### 创建和发送

创建请求必须有非空首条文本。`201` 后记录最近 Agent 并立刻进入上述恢复流程。

后续 `POST /messages` 的 `202` 表示消息已被服务端持久化接受，但前端不在稳定消息列表中制造没有 `business_seq` 的乐观消息。提交成功后清空 composer，消息本身只在 SSE 或历史恢复中出现。提交期间禁用重复提交；运行中的 Agent 禁用后续发送。

### 恢复和取消

本页发起的运行在收到 `Started` 后显示“取消”。刷新后若 `AgentView.status` 为 `running`，界面显示“继续运行”，让用户显式调用 `/resume`。

- `/resume` 成功后等待 SSE；返回 `resume_not_running` 时重新读取 AgentView，而不把它显示为不可恢复的失败。
- `/cancel` 返回 `resume_required` 时转为继续运行提示；其他成功取消等待 `Cancelled` SSE event 更新界面。
- `agent_busy` 保持当前状态并提示该 Agent 正在运行，不自动重试。

### 工具审批

审批卡显示工具名、JSON 参数、读取/写入属性和风险等级。点击批准或拒绝后，按钮进入提交中；`204` 不直接移除卡片，等待 `ToolApprovalResolved` 事件。`approval_not_active` 或其他冲突时触发状态重读，避免本地与运行时不一致。

## 错误呈现

- `invalid_request`、`invalid_message`、`message_too_large`、`invalid_agent_template`、`model_not_configured`、`tool_not_available`：在对应表单原位显示，保留用户草稿。
- `agent_not_found`：显示失效的最近会话入口和移除操作。
- `resume_required`、`resume_not_running`、`agent_busy`、`approval_not_active`：显示可操作的运行状态提示，而不是通用 toast。
- `store_unavailable`、`event_stream_unavailable` 与网络错误：保留现有对话，显示可重试连接或命令操作。
- 错误正文从 API 取得稳定 `code` 和安全 `message`；不在客户端展示未知 payload、prompt、reasoning、工具参数以外的私密原始错误数据。

## UI 范围

`ChatWorkspace` 继续保留现有 `data-slot="chat-main"`、高度、居中消息 scroller 和 composer 位置。历史栏改为 localStorage recent agents；它不向服务端请求不存在的 Agent 列表。

消息区域在不改变画布布局的前提下，增加：

- assistant 流式文本与 reasoning 的临时行；
- 可折叠的工具过程详情；
- 内嵌工具审批卡；
- 运行状态、继续运行、取消与重新连接操作；
- 失效会话与恢复/连接错误的明确状态。

首版沿用现有 shadcn/Base UI 外观；Markdown、代码块和 AI Elements 视觉组件均不在本轮接入范围。

## 验证

新增 `vitest`，用纯 TypeScript fixtures 验证：

- 历史与 SSE 稳定消息的 `(agent_id, business_seq)` 去重及排序；
- SSE 先连、AgentView 固定边界分页、缓冲排空的恢复顺序；
- `after_cursor` 恢复和 `410` 后完整恢复；
- LLM 文本、reasoning、工具参数/结果和终态投影；
- 审批请求、提交中和 resolved 生命周期；
- 切换 Agent 时旧连接和晚到异步结果不污染新 Agent；
- API URL、请求 body、常见 HTTP error code 与 SSE JSON 解码。

实施完成后运行：

```bash
pnpm --dir wyse-web typecheck
pnpm --dir wyse-web test
pnpm --dir wyse-web build
```

## 完成标准

- Web 不包含 AI SDK、assistant-ui 或任何其运行时/协议依赖。
- 当前浏览器可创建、重新打开、发送、恢复、取消和审批一个已知 Agent。
- 刷新和 cursor 过期都不会丢失已提交消息或把 transport cursor 当成 business sequence。
- 页面布局仍满足 `wyse-web/AGENTS.md` 的 Chat 画布约束。
- reducer/API client 有针对上述协议不变量的自动化测试。
