# 协议

## 禁令
| 禁令 | 原因 |
| --- | --- |
| 不经允许额外添加 trait 定义 | 代码的目的是简洁高效，不经考量的 trait 只会增加维护成本 |

## 事件 Envelope

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `run_id` | `RunId` | 一次 workflow run 的身份。 |
| `seq` | `u64` | 同一个 run 内单调递增的事件序号。 |
| `timestamp` | `DateTime<Utc>` | 事件创建时间。 |
| `source` | `EventSource` | 事件归属位置。 |
| `event` | `RuntimeEvent` | 类型化事件内容。 |
| `metadata` | `map` | 运行时附加元数据。 |

## RunId

| 项目 | 规则 |
| --- | --- |
| 格式 | `RunId` 使用 UUIDv7。 |
| 生成位置 | run 创建时生成一次。 |
| 分布式语义 | `RunId` 是多实例 agent os 中一次 workflow run 的全局身份。 |
| 关联范围 | event、agent 输出、tool call、checkpoint、resume 都必须使用同一个 `run_id`。 |
| 排序用途 | UUIDv7 只提供跨实例的粗略时间有序；同一个 run 内的精确顺序仍以 `seq` 为准。 |

## 分布式规则

| 项目 | 规则 |
| --- | --- |
| 关联主键 | `run_id` 是 event log、checkpoint、agent 状态、tool call 状态的主关联键。 |
| 实例无关 | 任意实例接手同一个 run 时，必须继续使用原 `run_id`。 |
| 顺序来源 | 同一个 run 内的事件顺序只能由 `seq` 决定。 |
| 幂等写入 | event log 写入以 `(run_id, seq)` 去重。 |
| checkpoint 分片 | checkpoint 存储和查询都以 `run_id` 为第一查询条件。 |
| 前端订阅 | 前端只按 `run_id` 订阅一次 workflow run 的完整事件流。 |

## Event Source

| Source | 说明 |
| --- | --- |
| `run` | 整个 workflow run。 |
| `node` | workflow node。 |
| `agent` | workflow node 内部运行的 agent。 |

## Runtime Events

| Event | Source | 说明 |
| --- | --- | --- |
| `run_started` | `run` | run 开始。 |
| `run_finished` | `run` | run 成功结束。 |
| `run_failed` | `run` | run 失败。 |
| `run_cancelled` | `run` | run 被取消。 |
| `node_started` | `node` | node 开始执行。 |
| `node_output` | `node` | 普通 node 产生输出。 |
| `node_finished` | `node` | node 执行完成。 |
| `node_failed` | `node` | node 执行失败。 |
| `llm_call_started` | `agent` | 一次 LLM 请求开始。 |
| `llm_call_finished` | `agent` | 一次 LLM 请求完成。 |
| `llm_call_failed` | `agent` | 一次 LLM 请求失败。 |
| `text_delta` | `agent` | LLM call 普通文本增量；`role` 表达 `system`、`user`、`assistant` 或 `tool`。 |
| `reasoning_delta` | `agent` | LLM call reasoning 增量。 |
| `tool_call_started` | `node` / `agent` | tool call 开始。 |
| `tool_call_delta` | `node` / `agent` | tool call 参数增量。 |
| `tool_call_finished` | `node` / `agent` | tool call 完成。 |
| `tool_call_failed` | `node` / `agent` | tool call 失败。 |
| `plan_updated` | `agent` | agent 可见计划发生变化。 |

## Checkpoint

| 项目 | 规则 |
| --- | --- |
| Event log | 按 `(run_id, seq)` 追加保存每个 envelope。 |
| Checkpoint | 按 `run_id` 保存从事件投影出的 run/node/agent 当前状态。 |
| Resume | 先加载最新 checkpoint，再 replay 后续事件。 |
| Frontend reconnect | SSE 从 `Last-Event-ID + 1` 继续。 |

## SSE

| 字段 | 值 |
| --- | --- |
| `id` | `seq` |
| `event` | runtime event type |
| `data` | 完整 `StreamEnvelope` JSON |

## 使用规则

| 规则 | 说明 |
| --- | --- |
| `source` 只表达归属 | 用来定位事件属于 run、node 还是 agent。 |
| `event` 表达发生了什么 | 不用 `metadata`、`source` 或字符串状态重复表达事件类型。 |
| 一次 LLM 请求只用 `llm_call_id` 关联 | 不记录 `model_id`，不引入 `message_id`，文本顺序由 `seq` 决定。 |
| user、assistant、tool 普通文本共用 `text_delta` | `text_delta.role` 表达文本归属；reasoning 使用独立 `reasoning_delta`。 |
| tool 没有独立 source | `tool_call_*` 使用 `node` 或 `agent` source；`llm_call_id`、`call_id`、`tool_id` 放在事件 data 中。 |
| agent 是特殊 node | 生命周期仍用 `node_started` / `node_finished` / `node_failed`；内部输出用 `agent` source。 |
| 普通 node 输出用 `node_output` | 不新增 `task` 事件；需要用户可见计划时才用 `plan_updated`。 |

## Metadata 禁令

| 禁令 | 原因 |
| --- | --- |
| 禁止放业务 payload | 业务数据必须进入类型化 event data。 |
| 禁止放事件状态 | 状态必须由 event type 或类型化字段表达。 |
| 禁止放 ID 主字段 | `run_id`、`node_id`、`agent_id`、`llm_call_id`、`tool_id`、`call_id` 必须放在明确字段中。 |
| 禁止放 secret | 不记录 token、凭据、密钥或原始敏感数据。 |
| 禁止让前端依赖 metadata 渲染核心 UI | 前端核心渲染只能依赖 envelope、source 和 event data。 |

| 允许项 | 用法 |
| --- | --- |
| debug hint | 临时诊断信息，不参与业务语义。 |
| provider metadata | 外部 provider 返回的非敏感附加信息。 |
| trace correlation | trace/span/request correlation id。 |
