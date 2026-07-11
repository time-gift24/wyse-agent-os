# wyse-api 约定

- 只有在 agent/run 的必要状态和输入已持久化后，创建或消息接口才能返回已接受；失败不得留下可被接受为成功的半成品。
- hosted-agent registry 的锁只保护内存映射访问。文件系统、Store、NATS、provider 和 agent 的异步工作必须在锁外完成。
- Store 是 agent 状态、消息历史和启动恢复的持久化真相源；NATS/JetStream 只负责事件分发与重放，不能代替 Store。
- SSE 使用传输序号 cursor：响应写入 `id`，恢复时 `Last-Event-ID` 优先于 `after_cursor`，过期 cursor 必须显式报错。
- API 不引入 Session，也不以进程内 cache 作为恢复来源。启动时必须从持久化 definition 和 Store 完整重建 registry；恢复失败不得返回部分 registry。
