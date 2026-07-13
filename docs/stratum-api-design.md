# `stratum-api` Agent 对话 API 设计

## 状态

本文记录 `stratum-api` 首版 Agent 对话 API 的批准设计。API 根据磁盘上的 Agent 模板创建
带首条用户消息的 Agent runtime，并用服务端生成的 UUIDv7 `agent_id` 管理后续对话；
不引入 Session。

## 目标

- 新增可直接运行的 `stratum-api` crate，使用 Axum 提供 HTTP 与 SSE。
- 新增共享的 `stratum-config` crate，统一解析 provider、model、Agent、API 和 NATS 配置。
- 从 `templates/{agent_name}.toml` 创建 Agent，并把解析后的完整定义固化到该实例历史。
- 创建请求必须同时提交首条用户消息，不产生可见的空 Agent。
- 提供 Agent 状态、完整消息历史、发送消息、恢复、取消和工具审批接口。
- 使用 AgentStore 恢复永久完整消息，使用 NATS 恢复近期过程事件并继续实时事件。
- 保持 Stratum 原生事件协议；前端框架适配留在 Web 客户端。

## 非目标

- Session、Agent 列表、删除、配置修改或模板管理 API。
- 模板继承、模板热重载、动态第三方工具或创建幂等键。
- 分布式 writer lease、多写者输入队列或跨进程 registry。
- assistant-ui、AI SDK Data Stream 或其他前端库专用协议。
- 服务端对话 snapshot、稳定消息 cache 命中计算或恢复控制帧。
- 用户、登录、ACL、远程多租户和共享授权。
- workflow、run 或 node 级事件订阅。
- 新增 Store、Filesystem、repository、service、factory 或 manager 抽象。

## 核心决策

### Agent 定义与实例身份分离

- `agent_name` 是 `templates/` 下可复用定义的文件名。
- `agent_id` 是每次创建时由服务端生成的 UUIDv7，只标识一个持久化 runtime。
- 相同 `agent_name` 可以创建多个互相独立的 `agent_id`。
- 模板修改只影响之后创建的实例；已有实例始终使用创建时固化的定义。

### Agent 原生协议

历史接口直接返回现有 `HistoryPage`，SSE 的 data 直接使用现有
`StreamEnvelope`。API 不把 Stratum 事件转换成 UI runtime 的消息格式。

### Store 与 NATS 各司其职

- AgentStore 是完整 user、assistant、tool 消息的永久事实来源。
- NATS JetStream 是近期类型化过程事件和实时事件流。
- 前端对两个来源进行投影，并按 `(agent_id, business_seq)` 去重稳定消息。
- 首版允许重复读取 NATS 中也存在的稳定消息，不为节省本地文件读取增加服务端恢复
  状态机。只有实际测量证明 AgentStore 恢复是瓶颈时，才增加 cache fast path。

## workspace 结构

```text
crates/stratum-config/
├── Cargo.toml
└── src/
    ├── error.rs
    └── lib.rs

crates/stratum-api/
├── Cargo.toml
├── AGENTS.md
├── src/
│   ├── api.rs
│   ├── error.rs
│   ├── host.rs
│   ├── lib.rs
│   └── main.rs
└── tests/
    └── api.rs
```

`stratum-config` 只定义严格的外部配置协议、解析和校验。它不创建 provider、Agent、Store、
NATS 连接或 tool registry。

`stratum-api` 的职责保持扁平：

- `api.rs`：路由、HTTP DTO、错误映射和 SSE 编码。
- `error.rs`：类型化 host/API 错误；不与具体实现混放。
- `host.rs`：装配 LocalFilesystem、Store、NATS、provider、tool registry 和 Agent。
- `lib.rs`：导出 host、router 和 `serve`。
- `main.rs`：读取配置、安装 application tracing subscriber、启动和优雅关闭。
- `tests/api.rs`：通过 InMemoryEventStreamBus 验证 HTTP 边界。

首版不增加 `recovery.rs`、独立 DTO 模块、Makefile 或第二份 NATS compose；真实
JetStream 行为继续由 `stratum-infra` 的集成测试负责。

## 统一配置

根 `config.toml` 使用以下结构：

```toml
[agent]
storage_root = "./.stratum/agents"

[llm]
default = "deepseek:deepseek-v4-flash"

[llm.deepseek]
api_key = "replace-with-your-api-key"
models = [
  "deepseek-v4-flash",
  "deepseek-v4-pro",
]

[llm.openai]
api_key = "replace-with-your-api-key"
models = [
  "gpt-4.1-mini",
  "gpt-4.1",
]

[api]
bind = "127.0.0.1:8080"
allowed_origins = ["http://localhost:5173"]

[nats]
url = "nats://localhost:4222"
stream_name = "AGENT_EVENTS"
subject_prefix = "events.agent"
replicas = 1
max_age_seconds = 604800
max_bytes = 1073741824
max_messages = 1000000
```

规则：

- 不再使用 `[stratum]`、顶层 `[openai]` 或顶层 `[deepseek]`。
- `llm.default` 是完整的 `provider:model`，并且必须存在于对应 provider 的 `models`
  列表。
- provider 的 `models` 必须非空且不能重复；API key 必须非空。
- `[agent]` 和 `[llm]` 是共享配置的必填 section；`stratum-api` 还要求 `[api]` 和
  `[nats]` 存在。
- `[nats]` 映射为现有 `NatsEventStreamBusConfig` 并复用 retention 校验。
- `api.bind` 默认使用 loopback；只有显式配置时才监听非 loopback 地址。
- `api.allowed_origins` 为空时不安装 CORS layer；不允许 `*`。
- 根配置及所有子 section 严格拒绝未知字段。
- provider API key 不进入 tracing、definition 文件或 API 错误正文。

`stratum-config` 是 API 与 Stratum REPL 的两个真实消费者共享的文件协议，不承载运行时
抽象。REPL 改为读取 `[agent]` 和 `[llm]`，并使用 `llm.default` 替代原来的
`stratum.model`。REPL 写入同一 storage root 时也必须使用 `history/{agent_id}` 并保存
完整 definition，避免产生 API 无法恢复的另一种磁盘格式；本阶段不扩展 REPL 的交互
功能。

## Agent 模板与固化定义

目录固定从 `agent.storage_root` 派生，不增加单独的 templates/history 路径配置：

```text
.stratum/agents/
├── templates/
│   └── coding-agent.toml
└── history/
    └── 01900000-0000-7000-8000-000000000000/
        ├── definition.toml
        ├── agent.json
        └── messages/
```

模板文件名就是 `agent_name`。名称必须匹配
`[A-Za-z0-9][A-Za-z0-9_-]{0,63}`，因此不能包含路径分隔符、`.` 或 `..`。

示例模板：

```toml
prompt = """
你是一个负责检查 Rust agent runtime 的工程助手。
"""

tools = ["echo"]
model = "deepseek:deepseek-v4-pro"
```

模板规则：

- `prompt` 必填，去除首尾空白后不能为空。
- `tools` 必填，允许空列表但不能重复。
- `stratum-config` 只解析为 `ToolName` 并检查重复；`stratum-api` 再根据当前真实 builtin
  catalog 校验可用性。首版 catalog 只有 `echo`，未知工具创建失败。
- `model` 可选；存在时必须是完整 `provider:model`，省略时使用 `llm.default`。
- 最终 model 必须存在于对应 provider 的 `models` 列表。

创建时把模板解析为完整 definition 并写入实例目录：

```toml
agent_name = "coding-agent"
model = "deepseek:deepseek-v4-pro"
tools = ["echo"]
prompt = """
你是一个负责检查 Rust agent runtime 的工程助手。
"""
```

definition 不保存 API key。重启只读取固化 definition，不重新套用当前模板或当前
`llm.default`。如果 definition 引用的 provider 凭据或 model 已从当前系统配置移除，
启动失败而不是悄悄替换模型。

## host 与运行时生命周期

```text
HostState
- agents: RwLock<HashMap<AgentId, Arc<HostedAgent>>>
- filesystem: Arc<LocalFilesystem>
- event_bus: Arc<dyn EventStreamBus>
- config: Arc<Config>

HostedAgent
- agent: Agent
- store: Arc<dyn AgentStore>
- needs_resume: AtomicBool
```

registry lock 只用于短时查找和插入，不在锁内执行文件 I/O 或 `.await`。NATS bus 和
LocalFilesystem 是 host 共享依赖；每个 Agent 拥有独立 Store、Agent runtime 与恢复
标记。

启动顺序：

1. 读取并严格校验 `config.toml`。
2. 创建 `storage_root/templates` 和 `storage_root/history`。
3. 创建一个 NATS EventStreamBus。
4. 扫描 `history/*`，严格解析 UUIDv7 目录、`definition.toml` 和 AgentStore 状态。
5. 按 definition 的完整 model 构造绑定该 model 的 provider。
6. 按 definition 的 tools 校验 builtin catalog，并构造 `RequireApproval` 的
   BuiltinToolRegistry。
7. 用 `StoreEventStreamBus`、Agent builder 和固化的 name/prompt 构造 Agent。
8. 状态不是 `running` 时调用 `Agent::load_history()`；`running` 时设置
   `needs_resume = true`，等待显式 `/resume`。
9. 全部历史恢复成功后才启动 Axum listener。

任一完整历史损坏时启动失败，不静默跳过。host 是单进程、单逻辑 writer，不声称提供
跨进程写入协调。

## HTTP API

| 方法和路径 | 行为 | 成功响应 |
| --- | --- | --- |
| `POST /v1/agents` | 根据模板和首条用户消息创建 Agent | `201 AgentCreated` |
| `GET /v1/agents/{agent_id}` | 读取持久化 Agent 状态 | `200 AgentView` |
| `GET /v1/agents/{agent_id}/messages` | 分页读取完整消息 | `200 HistoryPage` |
| `POST /v1/agents/{agent_id}/messages` | 发送用户文本并启动 turn | `202 RunAccepted` |
| `GET /v1/agents/{agent_id}/events` | retained + live Agent SSE | `200 text/event-stream` |
| `POST /v1/agents/{agent_id}/resume` | 恢复未完成 turn | `202 RunAccepted` |
| `POST /v1/agents/{agent_id}/cancel` | 请求取消当前进程的 active turn | `202` |
| `POST /v1/agents/{agent_id}/approvals/{approval_id}` | 提交审批决定 | `204` |

### 创建 Agent

请求必须同时包含模板名和首条用户文本：

```json
{
  "agent_name": "coding-agent",
  "text": "检查 event stream bus 的恢复逻辑"
}
```

`text` 去除首尾空白后不能为空，整个请求 body 上限为 64 KiB。所有输入校验必须在创建
目录前完成。成功返回 `Location: /v1/agents/{agent_id}`：

```json
{
  "agent_id": "01900000-0000-7000-8000-000000000000",
  "agent_name": "coding-agent",
  "run_id": "01900000-0000-7000-8000-000000000001"
}
```

创建流程：

1. 安全读取 `templates/{agent_name}.toml` 并解析完整 definition。
2. 生成 UUIDv7 `agent_id` 并创建 `history/{agent_id}`。
3. 写入 definition，初始化 FilesystemAgentStore 并构造 Agent。
4. 用首条用户消息调用 `Agent::run_turn()`。
5. 用户消息到达 AgentStore 持久化边界后，把 HostedAgent 插入 registry。
6. 返回 `201`、`agent_id` 和 `run_id`。

首条消息持久化失败时清理本次新建目录，不加入 registry，也不向调用方暴露
`agent_id`。消息已经持久化后，即使 provider 随后失败，也保留这个有效的非空对话，
失败通过 `AgentEvent::Failed` 发布。

### durable run acceptance

现有 `Agent::run_turn()` 调整为：

1. 建立 active guard、run/turn ID 与取消通道。
2. 在返回 `RunId` 前同步发布 `AgentEvent::Started` 和首条
   `AgentEvent::Message`。
3. `StoreEventStreamBus` 先提交状态和完整用户消息，再 best-effort 转发 NATS。
4. 只有上述持久化成功后才 spawn provider/tool continuation。
5. 持久化失败时释放 active 状态并返回类型化错误。

若 preamble 在 Started 已提交后失败，API 对已有 Agent 保守设置 `needs_resume`；后续先
从 Store 确认状态，再恢复或清除该标记，不能直接开始第二个 turn。创建中的 Agent 不会
进入 registry，并清理本次目录。

因此普通 `POST /messages` 返回 `202` 时，用户消息也已进入 AgentStore，而不是仅创建了
后台任务。客户端仍不能提交 assistant、tool 或 system 消息。

### AgentView

`AgentView` 不暴露持久化 schema 版本和 `next_iteration`：

```json
{
  "agent_id": "01900000-0000-7000-8000-000000000000",
  "agent_name": "coding-agent",
  "status": "running",
  "run_id": "01900000-0000-7000-8000-000000000001",
  "turn_id": "01900000-0000-7000-8000-000000000002",
  "usage": {
    "prompt_tokens": 0,
    "completion_tokens": 0,
    "total_tokens": 0
  },
  "last_seq": 1,
  "updated_at": "2026-07-11T12:00:00Z"
}
```

### 历史分页

查询参数直接映射 `HistoryQuery`：

```text
GET /messages?after_seq=0&limit=100
GET /messages?after_seq=100&through_seq=243&limit=100
```

第一次省略 `through_seq`，Store 返回当前固定边界；后续页面带回同一边界。响应直接
使用 `HistoryPage`。

### SSE

```text
GET /events?replay=all
GET /events?replay=new
GET /events?after_cursor=4281
Last-Event-ID: 4281
```

优先级：

1. `Last-Event-ID` 使用 `ReplayStart::After`。
2. 否则 `after_cursor` 使用 `ReplayStart::After`。
3. 否则 `replay=new` 使用 `ReplayStart::New`。
4. 其他情况默认 `ReplayStart::All`。

`after_cursor` 用于页面刷新或 Agent 切换后从客户端持久化 cursor 恢复。普通事件：

```text
id: <transport cursor>
event: <AgentEvent::event_type()>
data: <完整 StreamEnvelope JSON>
```

- cursor 过期时在建立 SSE body 前返回 `410 cursor_expired`。
- 已建立流发生 delivery/decode 错误时发送一次 `stream_error`，然后关闭。
- keep-alive 每 15 秒发送 comment，不产生业务事件。
- 不定义 `recovery_started`、`history_page` 或 `recovery_complete` SSE 控制事件。

### Web 恢复契约

1. 先连接 `/events?replay=all`，暂存 retained 和随后到达的 live events。
2. 读取 `GET /agents/{agent_id}`，取得 `last_seq` 和状态。
3. 使用固定 `through_seq = last_seq` 分页读取 `/messages`。
4. 按 `business_seq` 建立历史，并与暂存事件合并。
5. 对稳定消息按 `(agent_id, business_seq)` 去重。
6. 完成初始投影后继续使用同一条 SSE。
7. 保存 transport cursor；页面刷新后通过 `after_cursor` 恢复。
8. 收到 `410` 时清除 cursor，重新执行完整恢复。
9. 状态为 `running` 且当前进程 `needs_resume` 时，由用户决定是否调用 `/resume`。

SSE 先于历史分页建立，因此分页期间发生的事件不会落入窗口。`business_seq` 只排序
完整消息，transport cursor 只排序 NATS 事件，两者不能互换。

### 运行控制

- `needs_resume` 为 true 时发送新消息或取消均返回 `resume_required`。
- `/resume` 只接受持久化 `running` 的 Agent；恢复成功后清除 `needs_resume`。
- 其他情况下 `/cancel` 调用现有幂等 `Agent::stop()` 并返回 `202`。
- approval ID 必须属于当前 active turn；无 active approval 时返回冲突。
- 运行冲突复用现有 Agent 错误，不增加 `Agent::is_active()`。

## 错误协议

```json
{
  "error": {
    "code": "resume_required",
    "message": "agent has an unfinished persisted turn"
  }
}
```

| HTTP 状态 | 稳定错误 code |
| --- | --- |
| `400` | `invalid_request`, `invalid_agent_name`, `invalid_message`, `invalid_cursor`, `invalid_history_query` |
| `404` | `agent_not_found`, `agent_template_not_found` |
| `409` | `agent_busy`, `resume_required`, `resume_not_running`, `approval_not_active` |
| `410` | `cursor_expired` |
| `413` | `message_too_large` |
| `422` | `invalid_agent_template`, `model_not_configured`, `tool_not_available` |
| `503` | `store_unavailable`, `event_stream_unavailable` |
| `500` | `agent_initialization_failed`, `internal_error` |

持久化格式损坏、身份不匹配和不变量破坏映射为 `500`，不是可重试的 `503`。错误正文
不包含 host path、消息正文、prompt、reasoning、工具参数、provider payload 或 secret。

## 背压、可观测性与安全

- API 不增加恢复 buffer；Axum response stream 直接轮询 EventStream。
- registry 使用短时 `RwLock`，不在 `.await` 期间持有 guard。
- shutdown 使用 `CancellationToken` 和 Axum graceful shutdown。
- library 只发 tracing events；`main.rs` 安装 application subscriber。
- 请求 span 只记录 route、method、status、agent_id、agent_name、run_id、cursor 和耗时。
- 错误只在最终处理边界记录一次。
- 不记录消息、prompt、reasoning、工具参数/结果、API key 或原始凭据。
- 默认监听 loopback；CORS 只允许配置中的精确 origin。
- 输入 body limit 为 64 KiB。
- 首版没有远程身份认证；非 loopback 部署必须由上层可信网络边界保护。

## 依赖

新增 workspace 依赖仅限实际使用项：

- `axum`：router、JSON、SSE 和 server。
- `tower-http`：CORS 和 request tracing。
- `tracing-subscriber`：只供 binary 安装 subscriber。

`stratum-config` 复用现有 Serde、TOML、`ModelId` 和 `ToolName`。其余复用现有 Tokio、
Futures、LocalFilesystem、AgentStore、NATS 和 Stratum crates。不增加 EventSource、SSE
parser、缓存、状态管理或动态插件依赖。

## 测试

### `stratum-config`

- 接受完整嵌套配置，拒绝所有层级的未知字段。
- 默认 model 必须属于对应 provider 的 models；model 列表非空且无重复。
- 模板省略 model 时使用系统默认，完整 model 可以覆盖默认。
- 拒绝非法 agent_name、空 prompt、重复工具和未配置 model。
- 固化 definition 不包含 provider secret。

### `stratum-agent`

- `run_turn` 返回前 Started 状态和首条用户消息已提交。
- 持久化失败时返回错误、释放 active guard且不 spawn continuation。
- 现有流式、resume、cancel、approval 和并发 turn 测试保持通过。

### `stratum-api`

使用 InMemoryEventStreamBus 和临时 LocalFilesystem 覆盖：

- 缺少或空白首条消息时不创建目录。
- 相同 `agent_name` 创建两次得到不同 UUIDv7。
- 未知 builtin tool 在创建边界返回 `tool_not_available`。
- 创建成功前用户消息已进入 Store；失败时清理目录且不加入 registry。
- 重启从固化 definition 恢复，不受模板后续修改影响。
- 多 Agent 的 Store、状态和 SSE 按 `agent_id` 隔离。
- `GET AgentView`、固定 barrier 历史分页和普通消息发送。
- `replay=all`、`replay=new`、`after_cursor` 和 header 优先级。
- cursor 过期返回 `410`；流内错误发送一次 `stream_error`。
- `needs_resume` 阻止发送与取消、resume 后清除标记、active turn 幂等 cancel，以及
  approval 冲突映射。
- body limit、稳定错误 code 和 CORS。

不在 `stratum-api` 重复 NATS restart、retention 和 cursor expiry 容器测试；这些行为继续由
`stratum-infra/tests/event_stream_bus_nats.rs` 验证。

### 验证命令

```bash
cargo fmt --check
cargo test -p stratum-config
cargo test -p stratum-agent
cargo test -p stratum-api
cargo test -p stratum-agent-builtin
cargo test -p stratum-infra
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets
```

## 完备性检查

| 能力 | 首版落点 |
| --- | --- |
| 根据定义创建 Agent | `POST /v1/agents` + UUIDv7 |
| 防止空 Agent | 创建请求必带首条消息，持久化后才返回 |
| 模板与旧实例隔离 | `history/{agent_id}/definition.toml` |
| 多模型配置 | provider models 列表 + 完整 `ModelId` |
| 发送后续用户消息 | `POST /messages` |
| 流式文本、reasoning、工具事件 | 原生 Agent SSE |
| 完整消息历史 | AgentStore 分页 |
| 实时无窗口恢复 | 先 SSE、后固定 barrier 历史 |
| 页面刷新 cursor 恢复 | `after_cursor` |
| 进程崩溃后的 turn 恢复 | `POST /resume` |
| 用户取消 | active turn 幂等 `POST /cancel`；未恢复 turn 返回 `resume_required` |
| 工具审批 | 模板 tool allowlist + approval endpoint |
| provider、store、NATS | 统一 `config.toml` |
| NATS cursor 过期 | `410` 后完整恢复 |

## 文档归档

实现完成且准备合入前，必须把最终稳定约定简洁归档到
`crates/stratum-api/AGENTS.md` 和 `crates/stratum-config/AGENTS.md`，并提醒维护者完成该检查；
不得提交 `docs/superpowers/` 过程文档。
