# Stratum runtime event protocol

本文描述当前 `stratum-core`、`stratum-store`、`stratum-infra` 和 `stratum-api`
共同实现的事件协议。Rust 类型是最终事实来源；修改序列化结构时必须同步更新本文和边界测试。

## 三种顺序不能混用

| 标识 | 所属层 | 语义 |
| --- | --- | --- |
| `business_seq` | `StreamEnvelope` | AgentStore 为完整 `AgentEvent::Message` 提交的单调业务序号；只用于稳定消息历史、分页和去重。其他事件不携带该字段。 |
| `EventCursor` | `EventRecord.cursor` / SSE `id` | retained transport（当前为 JetStream）分配的不透明重放位置；只用于订阅恢复。它不出现在 SSE `data` 的 envelope 内。 |
| `iteration` | `AgentEvent::IterationCompleted` | 某一 turn 已到达 durable boundary 的 loop iteration；用于推进持久化 frontier，不是消息序号或 transport cursor。 |

客户端不得比较、换算或互相替代这三个值。JetStream retention 可以使 cursor 失效，但不会
改变 AgentStore 中已经提交的 `business_seq`。

## StreamEnvelope

```json
{
  "business_seq": 7,
  "run_id": "<uuid-v7>",
  "timestamp": "2026-07-16T08:00:00Z",
  "source": { "type": "run" },
  "event": {
    "type": "agent",
    "data": {
      "agent_id": "<uuid-v7>",
      "event": {
        "type": "message",
        "data": {
          "turn_id": "<uuid-v7>",
          "message": { "role": "user", "content": { "type": "text", "data": "hello" } }
        }
      }
    }
  },
  "metadata": { "agent_name": "coding-agent", "turn_id": "<uuid-v7>" }
}
```

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `business_seq` | `u64`，可选 | 只出现在已提交的完整 Agent message 上；缺省时不序列化。 |
| `run_id` | `RunId` | 一次 workflow run 的 UUIDv7 身份；恢复同一 run 时保持不变。 |
| `timestamp` | UTC timestamp | envelope 创建时间，不提供严格排序保证。 |
| `source` | `EventSource` | 事件的运行时归属。 |
| `event` | `RuntimeEvent` | `type` + 可选 `data` 的类型化 payload。 |
| `metadata` | map，可选 | 非业务性的诊断/关联信息；空 map 不序列化。 |

`RuntimeEvent` 和其嵌套 event 使用 Serde 的 adjacent tagging：有 payload 的事件是
`{"type":"...","data":{...}}`，unit variant 只有 `type`。

## EventSource 与 RuntimeEvent

`EventSource` 当前有三种形式：

```json
{ "type": "run" }
{ "type": "node", "node_id": "..." }
{ "type": "agent", "node_id": "...", "agent_id": "..." }
```

`RuntimeEvent` 支持 run lifecycle、node lifecycle/output、直接 `llm`、嵌套 `agent` 和
`plan_updated`。Agent 对话 API 当前由 `ScopedAgentEventSink` 生成如下 envelope：

- `source` 是 `{ "type": "run" }`，因为该 host 目前没有 workflow node scope；
- `event` 是 `RuntimeEvent::Agent { agent_id, event: AgentEvent }`；
- `run_id`、`agent_id` 和嵌套 `turn_id` 分别承担自己的关联职责。

因此消费者不得假设 Agent event 一定使用 `EventSource::Agent`。归属过滤以协议中的明确
字段为准；当前 Agent SSE 已经由服务端按 `agent_id` 订阅隔离。

## AgentEvent

| `type` | 关键字段 | 语义与持久化边界 |
| --- | --- | --- |
| `started` | `turn_id` | turn 开始；Store 先持久化 `running`、run/turn/model config，再 best-effort 转发 retained stream。 |
| `message` | `turn_id`, `message` | 完整 user/assistant/tool message；Store 提交后获得 `business_seq`，再 best-effort 转发。流式 delta 不进入历史。 |
| `tool_approval_requested` | `approval_id`, `agent_name`, `call_id`, `tool_name`, `arguments`, `tool_kind`, `danger_level` | 工具尚未执行，active turn 等待决定。审批状态当前仅在进程内，不通过 checkpoint 恢复。 |
| `tool_approval_resolved` | `approval_id`, `decision` | 决定为 `approve` 或 `reject`；必须发生在执行之前。 |
| `tool_execution_started` | `turn_id`, `call_id`, `tool_name` | 工具已通过查找、校验与审批，执行即将开始。该事件必须获得 durable sink acknowledgement 后 loop 才 dispatch 工具；它本身没有 `business_seq`。 |
| `iteration_completed` | `turn_id`, `iteration`, `usage` | iteration 达到 durable boundary；Store 通过 CAS 推进 frontier 后再转发。它本身没有 `business_seq`。 |
| `finished` | `finish_reason`, `usage` | Store 先持久化 `finished` 状态和累计 usage。 |
| `failed` | `error_text`, `usage` | Store 先持久化 `failed` 状态和累计 usage。 |
| `cancelled` | `usage` | Store 先持久化 `cancelled` 状态和累计 usage。 |
| `llm` | `llm_call_id`, `event` | LLM telemetry 的嵌套投影；best-effort，不控制 loop 正确性。 |

`usage` 的稳定字段是 `input_tokens`、`output_tokens`、`total_tokens`。累计采用饱和加法。

### Durable 与 telemetry

基础 loop 发出两套本地事件：

- `DurableAgentEvent`：loop 必须等待 `DurableEventSink::append` 成功后才能越过对应边界；
- `AgentTelemetryEvent`：`TelemetryEventSink::emit` 是 best-effort，丢失、超时或发布失败不能
  改变业务结果。

`ScopedAgentEventSink` 负责补齐 run/agent/turn scope，并将它们投影为上表的
`RuntimeEvent::Agent`。当前 telemetry 映射为嵌套 `LlmEvent::{Started, TextDelta,
ReasoningDelta, ToolCallDelta, Finished}`，通过容量 256 的 bounded queue 非阻塞入队；
队满、worker 已关闭或 durable fence 淘汰旧 telemetry 时丢弃；丢失、发布失败和超时合计
每个 turn 最多告警一次。
Telemetry 与 durable 使用独立通道；durable 通道优先并等待发布确认，不会排在 telemetry
backlog 后面。Durable 到达时，worker 丢弃其之前尚未开始发布的 telemetry；正在发布的
telemetry 最多等待 100ms。入队顺序号与发送在同一临界区完成，worker 仍会通过 durable
fence 丢弃晚到的旧顺序号，因此旧 telemetry 不会在后续 durable message 或 terminal event
之后发布。`durable` 描述 loop 的
acknowledgement 契约；具体哪些状态进入 AgentStore，由 `StoreEventStreamBus` 的投影规则决定，
不能把所有 durable event 都理解为历史消息。

## LlmEvent

`AgentEvent::Llm.data.event` 复用 `LlmEvent`。协议类型包括 `started`、`finished`、`failed`、
`text_delta`、`reasoning_delta`、`tool_call_started`、`tool_call_delta`、
`tool_call_finished` 和 `tool_call_failed`。当前基础 loop 只投影上一节列出的子集；消费者
必须容忍未来合法变体，并以嵌套 `event.type` 判断 LLM 子事件。

## 历史与固定 barrier 恢复

AgentStore 是完整消息和 Agent 状态的事实来源。`HistoryPage` 只包含带
`business_seq` 的完整 `message` envelope：

1. 第一次请求省略 `through_seq`，服务端返回固定 `through_seq = L`；
2. 后续页使用同一个 `through_seq=L` 和上一页 `next_front_seq`；
3. `has_more=false` 后完成该固定范围；
4. 与恢复期间暂存的 SSE 合并时，对 `business_seq <= L` 的稳定消息去重；
5. 继续消费同一条 SSE，不把 transport cursor 当作历史页边界。

## SSE 编码与重放

Agent API 的普通 SSE frame：

```text
id: <EventRecord.cursor transport sequence>
event: <nested AgentEvent type, or outer RuntimeEvent type>
data: <完整 StreamEnvelope JSON>
```

- 对 `RuntimeEvent::Agent`，`event:` 使用嵌套 `AgentEvent::event_type()`，包括
  `tool_execution_started` 和 `iteration_completed`；其他 RuntimeEvent 使用外层 type。
- `id` 是 transport cursor，不是 `business_seq`；浏览器重连的 `Last-Event-ID` 表示“从该
  cursor 之后重放”。服务端不会手动执行 `+1`。
- 恢复优先级：`Last-Event-ID` > `after_cursor` > `replay=new|all`；未提供时默认 `all`。
- 过期 cursor 在建立 body 前返回 HTTP `410 cursor_expired`，客户端必须清除 cursor 并做
  完整固定-barrier 恢复。
- 建流后的 delivery/decode 失败发送一次无 `id` 的 `stream_error`，随后关闭。
- 每 15 秒的 keep-alive comment 不代表业务事件，也不能推进客户端 cursor。

## Metadata 约束

- 业务 payload、状态和主 ID 必须放在类型化 envelope/event 字段中；
- metadata 只允许非敏感 debug hint、provider 附加信息和 trace correlation；
- 不得记录 token、凭据、prompt、reasoning、工具参数/结果或其他敏感用户数据；
- 前端核心投影不得依赖 metadata。当前 scoped sink 写入的 `agent_name` / `turn_id` 只用于
  诊断兼容，嵌套类型化字段仍是事实来源。

## 演进规则

- 公共 enum 是 non-exhaustive；消费者必须对未知事件做安全 no-op 或明确降级，不能让 reducer
  返回无效状态；
- 新增 event 时同步更新 `event_type()`、serde 边界测试、SSE/前端 union 与本文；
- 不为假设中的多实现新增 trait、adapter、facade 或第二套事件 schema。
