# wyse-api 约定

- 只有在 agent/run 的必要状态和输入已持久化后，创建或消息接口才能返回已接受；失败不得留下可被接受为成功的半成品。
- hosted-agent registry 的锁只保护内存映射访问。文件系统、Store、NATS、provider 和 agent 的异步工作必须在锁外完成。
- Store 是 agent 状态、消息历史和启动恢复的持久化真相源；NATS/JetStream 只负责事件分发与重放，不能代替 Store。
- SSE 使用传输序号 cursor：响应写入 `id`，恢复时 `Last-Event-ID` 优先于 `after_cursor`，过期 cursor 必须显式报错。
- API 不引入 Session，也不以进程内 cache 作为恢复来源。启动时必须从持久化 definition 和 Store 完整重建 registry；恢复失败不得返回部分 registry。
- 新 turn 必须同时通过 Agent 的 persisted-running 检查和 Store 的 running-transition CAS 防线；内存 active 优先返回 busy，持久化 `running` 则只能显式 resume，任何新 run 都不得覆盖旧 run/turn。
- failed/cancelled turn 中已经持久化的完整 user、assistant、tool 消息属于后续上下文；同进程后续请求必须从 Store 刷新到与重启恢复相同的 history，流式 partial delta 不进入 history。
- `HostState` 持有共享 shutdown token。shutdown 关闭 admission 后结束 SSE，等待已准入请求，再 stop 所有 active Agent 并有界等待终态持久化；超时保留 durable `running`，由下次启动显式 resume。
- create、message 和 resume 在任何持久化或 provider I/O 前必须取得 atomic admission RAII；shutdown 先关闭 admission 并等待已准入请求归零，再 snapshot registry、stop Agent。关闭后的新 durable work 返回安全稳定的 503，且不得触碰 Store/history。
- HTTP 最终错误边界只记录一次安全的结构化 operational error；span 可记录 agent/run/cursor 等 ID，不得记录 message、prompt、tool args、secret 或 host path。
