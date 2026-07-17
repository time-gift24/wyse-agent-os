# `stratum-api` Agent 对话 API 实现契约

## 文档状态

本文是当前实现的参考契约，替代早期“首版批准设计”。过去的目标、计划目录结构和未实现
假设不再具有规范性。发生冲突时，以 `crates/stratum-api/src/api.rs`、`error.rs`、
`host.rs` 以及 `docs/PROTOCOL.md` 为准。

当前 API 是单进程 hosted-agent 服务：从严格的磁盘模板创建持久化 Agent，提供 HTTP
命令、固定 barrier 消息历史和 retained + live SSE。它不提供 Session、Agent 列表/删除、
模板写入、动态第三方工具、workflow/run/node 级订阅、身份认证或分布式 writer lease。

## 资源与路由

| 方法和路径 | 行为 | 成功响应 |
| --- | --- | --- |
| `GET /v1/models` | 列出已配置模型及其 parameter JSON Schema | `200 { "models": ModelDescriptor[] }` |
| `GET /v1/agent/templates` | 列出可创建的模板及解析后的默认模型配置 | `200 { "agents": AgentTemplateView[] }` |
| `POST /v1/agents` | 根据模板、首条消息和可选模型配置创建 Agent | `201 AgentCreated` + `Location` |
| `GET /v1/agents/{agent_id}` | 读取持久化 Agent 状态投影 | `200 AgentView` |
| `GET /v1/agents/{agent_id}/messages` | 分页读取固定范围完整消息 | `200 HistoryPage` |
| `POST /v1/agents/{agent_id}/messages` | 发送用户消息和可选模型配置 | `202 RunAccepted` |
| `GET /v1/agents/{agent_id}/events` | 订阅该 Agent 的 retained + live SSE | `200 text/event-stream` |
| `POST /v1/agents/{agent_id}/resume` | 恢复持久化 `running` turn | `202 RunAccepted` |
| `POST /v1/agents/{agent_id}/cancel` | 请求停止当前进程中的 active turn | `202`，无 JSON body |
| `POST /v1/agents/{agent_id}/approvals/{approval_id}` | 提交 `approve` 或 `reject` | `204`，无 JSON body |

注意模板列表路径当前是单数 `agent`：`/v1/agent/templates`。

所有 JSON request struct 拒绝未知字段，body 上限为 64 KiB。查询参数中的未知 key 当前未
统一拒绝；客户端不应依赖它们被忽略。

## 模型与模板

### ModelConfig

模型选择始终是 provider-scoped model 与 provider 参数的快照：

```json
{
  "model": "openai:gpt-4.1-mini",
  "parameters": {
    "temperature": 0.2
  }
}
```

`GET /v1/models` 返回：

```json
{
  "models": [
    {
      "model": "openai:gpt-4.1-mini",
      "parameters_schema": { "type": "object" }
    }
  ]
}
```

`POST /v1/agents` 和 `POST /messages` 都接受可选 `model_config`：

- 创建时省略：使用解析后的模板默认配置；
- 创建时提供：先按已配置 provider schema 校验，再将该快照持久化；
- 后续消息省略：沿用 Agent 当前持久化配置；
- 后续消息提供：作为该 turn 的配置并持久化为之后 turn 的当前配置。

模型必须已配置，参数必须通过对应 provider schema。API 不暴露第二套 default-parameter
格式。

### Agent template

模板位于 `agent.storage_root/templates/{agent_name}.toml`。名称匹配
`[A-Za-z0-9][A-Za-z0-9_-]{0,63}`。模板是 strict TOML：

```toml
prompt = "You are a careful assistant."
model = "openai:gpt-4.1-mini" # 可选；省略时使用 llm.default
tools = ["echo"]              # 可选；省略等价于 []
```

- `prompt` 必填，trim 后不能为空；
- `model` 可选，但最终 model 必须已配置；
- `tools` 可选、可以为空、不得重复；
- 当前 API host 的 builtin catalog 只有 `echo`，其他名称在创建/恢复组合边界失败；
- resolved definition 固化为 `agent_name`、`model`、`tools`、`prompt`，不包含 API key；
- 模板修改只影响之后创建的实例，已有实例从固化 definition 恢复。

`GET /v1/agent/templates` 每项只返回 `agent_name` 和解析后的 `model_config`，不返回 prompt、
tool allowlist 或 secret。

## 创建与消息接受

创建请求：

```json
{
  "agent_name": "coding-agent",
  "text": "检查恢复逻辑",
  "model_config": {
    "model": "openai:gpt-4.1-mini",
    "parameters": {}
  }
}
```

`model_config` 可省略；`text` trim 后必须非空。成功响应：

```json
{
  "agent_id": "<uuid-v7>",
  "agent_name": "coding-agent",
  "run_id": "<uuid-v7>"
}
```

创建和后续消息只有在必要 state 与 user message 已到达 Store durable boundary 后才返回
成功。Store 是状态与完整消息的事实来源；JetStream retained forwarding 失败不会回滚已
提交消息。创建失败时只有在能够确定 mutation 已结束且消息目录为空时才清理，不能为追求
“无残留”删除可能已提交的数据。

后续消息请求：

```json
{
  "text": "继续",
  "model_config": {
    "model": "openai:gpt-4.1-mini",
    "parameters": {}
  }
}
```

成功返回 `{ "run_id": "<uuid-v7>" }`。客户端不能提交 assistant、tool 或 system message。

## AgentView

```json
{
  "agent_id": "<uuid-v7>",
  "agent_name": "coding-agent",
  "status": "running",
  "model_config": {
    "model": "openai:gpt-4.1-mini",
    "parameters": {}
  },
  "run_id": "<uuid-v7>",
  "turn_id": "<uuid-v7>",
  "usage": {
    "input_tokens": 12,
    "output_tokens": 8,
    "total_tokens": 20
  },
  "last_seq": 3,
  "updated_at": "2026-07-16T08:00:00Z"
}
```

- `status` 的完整集合是 `idle | running | finished | failed | cancelled`；
- `run_id` 和 `turn_id` 是 nullable；
- usage 字段是 `input_tokens`、`output_tokens`、`total_tokens`，不是
  `prompt_tokens` / `completion_tokens`；
- `last_seq` 是最后提交的完整消息 `business_seq`；
- 不暴露持久化 `state_version` 或 `next_iteration`。

## 历史分页

```text
GET /v1/agents/{agent_id}/messages?after_seq=0&limit=100
GET /v1/agents/{agent_id}/messages?after_seq=100&through_seq=243&limit=100
```

第一次省略 `through_seq`，返回值固定本轮读取的 inclusive barrier。后续页面必须带回同一
`through_seq`，并用上一页 `next_front_seq` 作为新的 `after_seq`。`limit` 默认 100，
Store 最大值为 256。响应 `events` 只包含已提交完整 message envelope。

## SSE 与 Web 恢复

```text
GET /v1/agents/{agent_id}/events?replay=all
GET /v1/agents/{agent_id}/events?replay=new
GET /v1/agents/{agent_id}/events?after_cursor=4281
Last-Event-ID: 4281
```

优先级是 `Last-Event-ID` > `after_cursor` > `replay`，缺省为 `replay=all`。SSE `id` 是
transport cursor，`data` 是完整 `StreamEnvelope`；与 `business_seq` 的关系及新增
`tool_execution_started` / `iteration_completed` 事件见 `docs/PROTOCOL.md`。

推荐恢复顺序：

1. 先建立 `replay=all` SSE 并暂存事件；
2. 读取 `AgentView.last_seq = L`；
3. 使用固定 `through_seq=L` 分页读取消息；
4. 用 `(agent_id, business_seq)` 去重稳定消息，再应用暂存事件；
5. 保持同一 SSE 继续 live 消费并保存最近 transport cursor；
6. cursor 过期收到 `410 cursor_expired` 时清除 cursor，重新做完整恢复。

建流后的 transport/decode 失败发送一次 `stream_error` 后关闭；15 秒 keep-alive comment 不
推进 cursor。

## 运行控制与审批

- persisted `running` turn 必须显式 `/resume`，不能被新消息覆盖；
- `needs_resume` 时发送或取消返回 `resume_required`；
- `/resume` 只接受持久化 `running`，否则返回 `resume_not_running`；
- 同进程 active run 冲突返回 `agent_busy`；
- `/cancel` 对当前进程 active Agent 调用 `stop()`；审批等待也由 cancellation 关闭；
- approval 只属于当前 active turn。请求 body 是
  `{ "decision": "approve" }` 或 `{ "decision": "reject" }`；
- 没有 active approval 或 ID 不匹配返回 `approval_not_active`。

## 错误协议

```json
{
  "error": {
    "code": "resume_required",
    "message": "agent has an unfinished persisted turn"
  }
}
```

| HTTP 状态 | 当前稳定 code |
| --- | --- |
| `400` | `invalid_request`, `invalid_agent_name`, `invalid_message`, `invalid_cursor`, `invalid_history_query` |
| `404` | `agent_not_found`, `agent_template_not_found` |
| `409` | `agent_busy`, `resume_required`, `resume_not_running`, `approval_not_active` |
| `410` | `cursor_expired` |
| `413` | `message_too_large` |
| `422` | `invalid_model_parameters`, `invalid_agent_template`, `model_not_configured`, `tool_not_available` |
| `500` | `agent_initialization_failed`, `internal_error` |
| `503` | `service_unavailable`, `store_unavailable`, `event_stream_unavailable` |

错误正文不包含 host path、消息、prompt、reasoning、工具参数/结果、provider payload、API key
或原始凭据。服务端只在最终 HTTP 边界记录一次错误；span 只记录安全的 route/status/ID/cursor
和耗时字段。

## 配置与部署边界

- `[agent]` 和 `[llm]` 是共享必填配置；API 还要求 `[api]` 和 `[nats]`；
- `api.bind` 缺省为 `127.0.0.1:8080`；CORS 只允许显式精确 origin，拒绝 `*`；
- host 启动时从固化 definition 与 AgentStore 重建完整 registry，任何历史损坏都 fail closed；
- shutdown 关闭 durable-work admission、结束 SSE、drain 已准入请求，然后 stop active Agent；
- 当前没有远程身份认证，非 loopback 暴露必须由上层可信网络与认证边界保护。

## 维护检查

修改 API 时必须同步：

1. HTTP DTO、错误 mapping 和边界测试；
2. `docs/PROTOCOL.md`（若涉及 event/SSE）；
3. `crates/stratum-api/AGENTS.md` 与相关 crate `AGENTS.md`；
4. 前端 `stratum-api.ts` 的 status、usage、event union 与恢复投影。
