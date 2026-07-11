# wyse-ontology 约定

- Draft 存在固定虚拟目录 `/ontology/drafts/{name}.json`，只接受逻辑名称，不接受主机路径。
- 发布 revision 使用通过校验的 schema 固定字段顺序 JSON 的 SHA-256 摘要；revision 永远不可变。
- `online` 是运行中 schema tag。每次 Object/Link 写入均同时按请求 schema 与 `online` schema 校验。
- Object 保存完整 JSON values；Link 只保存类型与两个端点，不保存 values。
- MySQL migration 仅可由 `sqlx` CLI 执行，应用启动时不得执行 migration。
