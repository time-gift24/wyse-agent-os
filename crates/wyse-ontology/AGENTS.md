# wyse-ontology 约定

- Draft 存在固定虚拟目录 `/ontology/drafts/{name}.json`，只接受逻辑名称，不接受主机路径。
- 发布 revision 使用通过校验的 schema 固定字段顺序 JSON 的 SHA-256 摘要；revision 永远不可变。
- 仓储写入或读回 revision 时必须重新校验 schema 与其 canonical SHA-256 identity；不得信任调用方或持久化层提供的 digest。
- `publish_revision` 是发布校验与 revision 写入的唯一原子仓储边界；实现必须与所有 Object/Link 写入线性化，不能将快照校验和写入拆成两个独立操作。
- `online` tag 的移动必须在同一原子仓储边界内校验全部现存实例；Object/Link 写入必须在持久化时复核其服务层已验证的 online revision 仍为当前值。
- 发布 revision 或移动 `online` tag 时，必须使用候选 schema 重新校验全部现存 Link 的 source/target 类型与基数；不能只在新增或修改 Link 时校验基数。
- `online` 是运行中 schema tag。每次 Object/Link 写入均同时按请求 schema 与 `online` schema 校验。
- Object 保存完整 JSON values；Link 只保存类型与两个端点，不保存 values。
- MySQL migration 仅可由 `sqlx` CLI 执行，应用启动时不得执行 migration。
- 新的本地文件系统根目录可以没有 `/ontology/drafts`：列举 draft 返回空列表，首次创建 draft 时由 `FilesystemDraftStore` 按需创建固定目录；支持 CAS 但不支持目录 API 的后端保持原有行为。
