# wyse-ontology 约定

- Draft 存在固定虚拟目录 `/ontology/drafts/{name}.json`，只接受逻辑名称，不接受主机路径。
- 发布 revision 使用通过校验的 schema 固定字段顺序 JSON 的 SHA-256 摘要；revision 永远不可变。
- 仓储写入或读回 revision 时必须重新校验 schema 与其 canonical SHA-256 identity；不得信任调用方或持久化层提供的 digest。
- `publish_revision` 是发布校验与 revision 写入的唯一原子仓储边界；实现必须与所有 Object/Link 写入线性化，不能将快照校验和写入拆成两个独立操作。
- `online` 是运行中 schema tag。每次 Object/Link 写入均同时按请求 schema 与 `online` schema 校验。
- Object 保存完整 JSON values；Link 只保存类型与两个端点，不保存 values。
- MySQL migration 仅可由 `sqlx` CLI 执行，应用启动时不得执行 migration。
