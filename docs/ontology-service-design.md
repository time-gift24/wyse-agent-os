# Ontology Service 设计

## 状态

第一个端到端切片已实现：核心领域、Filesystem draft、MySQL 8 仓储、Axum router 与 `/ontology` 建模界面均有对应测试。Axum router 尚未组合进 Rust 应用 host，也尚未部署；前端当前连接配置的 Ontology API，未配置时使用演示数据。本文件记录当前实现边界，仍刻意独立于 memory、租户、workspace、授权和业务领域。

## 目标

- 提供一个通用 Ontology Service，覆盖 schema 与实例的完整 CRUD。
- 通过 `wyse-filesystem` 将可编辑 schema draft 保存为 JSON 文件。
- 将不可变、内容寻址的 schema revision 发布到 MySQL 8。
- 让运行时选择 draft、revision 或 tag 作为 schema reference。
- 让所有 schema reference 共享同一份 Object 与 Link 实例数据。
- 为未来图编辑器提供类型图，为未来前端提供分页实例表格。
- 通过 Axum 暴露 REST/JSON 接口。

## 非目标

- memory 专属模型或检索行为。
- 租户、workspace、角色或授权执行。
- SharedProperty、InterfaceType、ActionType、派生属性、自定义 value type 或业务主键语义。
- Object 实例图渲染、图布局持久化、动态属性索引或动态 SQL 建表。
- Git repository、Git 兼容对象、branch、merge 或 Git 工具链。
- 运行时数据库 migration、建表或改表。

## 核心模型

Service 只管理一个逻辑 Ontology。一个 schema 是包含 ObjectType 与 LinkType 定义的 `SchemaDocument`。

```text
SchemaDocument
|- ObjectType
|  `- PropertyType
`- LinkType

运行时数据
|- Object  -- object_type_id --> 所选 schema 中的 ObjectType
`- Link    -- link_type_id   --> 所选 schema 中的 LinkType
```

每个 schema resource、Object 和 Link 都由 Service 生成 UUID。已发布 revision 使用内容哈希作为身份。Object UUID 仅为内部身份；第一个切片不要求业务主键，也不包含领域专属唯一性规则。

### Schema 原语

ObjectType 有不可变 UUID、展示名称、描述与有序的 PropertyType。PropertyType 有不可变 UUID、名称、描述、一个值类型和 `required` 标记。支持的值类型是 `string`、`integer`、`number`、`boolean`、`datetime` 和 `json`。

LinkType 有不可变 UUID、名称、描述、源与目标 ObjectType UUID，以及一个基数：`one_to_one`、`one_to_many`、`many_to_one` 或 `many_to_many`。Service 在创建或修改 Link 时强制执行基数。`one_to_one` 对同一 LinkType 限制每个 source 和每个 target 各最多一条 Link；`one_to_many` 限制每个 target 最多一个 source，而 source 不限；`many_to_one` 限制每个 source 最多一个 target，而 target 不限；`many_to_many` 不限制任一端点。self-link 允许，只要上述计数通过。PATCH 可以替换 Link 的两个端点，但不能替换 LinkType；校验 PATCH 后的计数时排除该 Link 自己的 LinkId。

在同一个 schema 内，ObjectType 名称与 LinkType 名称必须唯一；PropertyType 名称只需在所属 ObjectType 内唯一。所有被引用的类型 UUID 必须存在于该 schema。

Object 值保存为 JSON 文档。Service 根据所选 schema 校验字段名、必填字段和值类型；MySQL 不推断也不拥有动态 schema 规则。Link 只由其类型与两个 Object 端点构成。

## Draft、revision 与 tag

Draft 使用 Git 式语义，但不依赖 Git。

```text
文件系统
`- /ontology/drafts/{draft_name}.json      可编辑 schema draft

MySQL
|- ontology_revisions                       不可变已发布 schema
|- ontology_tags                            可变 tag -> revision 映射
|- objects                                  共享 Object 实例
`- links                                    共享 Link 实例
```

Draft 名称是全局唯一的逻辑名称，不能包含路径分隔符。外部 API 只使用该名称，永不暴露主机路径或虚拟文件系统路径。

一个 draft 文件只包含一个 schema body。draft 名称由文件名提供，不参与可哈希 schema body。revision 的规范字节算法固定为：验证 schema 后，使用 serde struct 和有序 `Vec` 字段表示 SchemaDocument；schema 中不存在 map；使用 `serde_json::to_vec` 序列化；字段按 struct 声明顺序输出，数组保留其顺序；拒绝不受支持的非有限 JSON number（schema 本身不包含它）。revision ID 是这份规范字节的 SHA-256 内容哈希，并持久化同一份规范 JSON。发布相同 schema 内容是幂等操作。

已发布 revision 不可变。draft 可为空，也可从任意已发布 revision 复制。恢复历史 revision 时，只能复制为新的 draft；历史 revision 永远不会变为可编辑状态。tag 是指向已发布 revision 的可变名称。保留的 `online` tag 表示线上运行时使用的 schema。

Draft 变更和删除都携带期望的当前 draft digest。DraftStore 复用现有 `Filesystem::get`、`Filesystem::put` 与 `Filesystem::delete` compare-and-swap 操作；digest 过期时返回前置条件失败，不覆盖其他修改。

## 共享实例数据不变量

Object 与 Link 不会按 draft、revision 或 tag 复制或版本化。请求选择一个 schema reference，只用于解析类型和校验操作。

每次 Object 或 Link 创建、更新、删除都同时对以下 schema 校验：

1. 请求所选的 schema reference；
2. `online` tag 指向的 revision。

这让 draft 能用于运行时实验，同时阻止 draft 写操作破坏线上 schema 的数据契约。draft 只有在其完整 schema 能校验通过全部共享 Object 与 Link 后，才能成为已发布 revision。

删除仍有入边或出边 Link 的 Object 默认被拒绝。调用方可设定 `force=true`；Service 在同一事务中先删除相关 Link，再删除 Object。若共享数据仍依赖某个类型或 LinkType，则包含删除该定义的 draft 不能发布。

## MySQL 8 持久化

MySQL 8 是唯一持久化后端。全部数据库表均为静态表；创建 ObjectType 不会创建或修改 MySQL 表。

| 表 | 必需职责 |
| --- | --- |
| `ontology_revisions` | revision hash、规范 schema JSON、schema format version、创建时间 |
| `ontology_tags` | 唯一 tag 名称、目标 revision hash、更新时间 |
| `objects` | UUID、逻辑 ObjectType UUID、JSON 值、乐观锁版本、时间戳 |
| `links` | UUID、逻辑 LinkType UUID、源/目标 Object UUID、乐观锁版本、时间戳 |

`objects` 按 `object_type_id` 建索引。`links` 分别按 `link_type_id`、`source_object_id` 和 `target_object_id` 建索引；其端点列通过外键引用 `objects`。逻辑 schema UUID 有意不设数据库外键，因为它们保存在不可变 schema JSON 中。

动态属性的筛选、排序和索引暂不实现。后续只有在工作负载证明存在热点字段时，才通过新的显式 migration 添加 MySQL generated column 索引，绝不由用户控制动态 DDL。

Migration 存放在 `wyse-ontology-mysql/migrations/`，仅由 SQLx CLI 手动管理。应用启动时不调用 `sqlx::migrate!`，不建表，也不修改数据库 schema。

## HTTP API

所有 REST/JSON 接口位于 `/v1`。请求恰好从 `draft`、`revision` 或 `tag` 中选择一个 schema；HTTP 解析后将其转换为强类型 `SchemaRef`。

| 区域 | 接口 |
| --- | --- |
| Draft | `POST/GET /ontology/drafts`、`GET/DELETE /ontology/drafts/{name}`、`POST /ontology/drafts/{name}/validate` |
| Draft schema | ObjectType、PropertyType 与 LinkType 的嵌套 `POST/PATCH/DELETE` 路由 |
| Revision | `POST /ontology/drafts/{name}/publish`、`GET /ontology/revisions`、`GET /ontology/revisions/{id}` |
| Tag | `GET/PUT/DELETE /ontology/tags/{name}`；`online` 是保留的部署 tag |
| 类型图 | `GET /ontology/graph?draft=...`、`?revision=...` 或 `?tag=...` |
| Object | `POST /objects`、分页 `GET /objects`、`GET/PATCH/DELETE /objects/{id}` |
| Link | `POST /links`、分页 `GET /links`、`GET/PATCH/DELETE /links/{id}` |

类型图响应是与展示层无关的投影：

```json
{
  "schema_ref": { "kind": "tag", "name": "online" },
  "nodes": [{ "id": "...", "label": "Customer", "property_count": 3 }],
  "edges": [{ "id": "...", "label": "places", "source": "...", "target": "...", "cardinality": "one_to_many" }]
}
```

前端使用图节点的 ObjectType UUID 请求传统的、基于 cursor 分页的 Object 表格。第一个切片仅支持按系统字段排序和分页。

`POST /objects` 与 `POST /links` 不要求客户端携带实例版本，并创建 version 为 1 的实例。`PATCH` 与 `DELETE` Object 或 Link 时，客户端必须在请求 body 或 query 中携带当前实例版本。draft replacement 必须携带当前 draft digest。版本或 digest 不匹配时返回 `412 Precondition Failed`。重复资源、不兼容引用、基数冲突和未强制的引用删除返回 `409 Conflict`。非法值或 schema 定义返回带结构化诊断的 `422 Unprocessable Entity`。

## Crate 边界

- `wyse-ontology`：领域 newtype 与类型、规范 schema codec、draft 访问协议、校验与 Service 用例；不依赖 Axum 或 SQLx。
- `wyse-ontology-mysql`：SQLx/MySQL 持久化实现与 migration。
- `wyse-ontology-api`：Axum `router()`、HTTP DTO 与错误映射。未来的应用宿主负责提供 MySQL pool 和虚拟文件系统。

现有 `wyse-filesystem::Filesystem` 是 draft 文件边界。Ontology Service 将逻辑 draft 名称映射到固定虚拟 draft 目录，永不接受主机路径。

## 错误与可观测性

`OntologyError` 使用 `thiserror`。它区分 draft、schema、revision/tag、乐观并发、基数、删除引用、序列化、文件系统与存储失败。仅 Axum 边界将其映射为 HTTP 响应。结构化 tracing 记录资源 ID、schema-ref 类型和错误类别，但不记录 Object JSON 值。

## 验证

- 单元测试覆盖规范序列化、内容哈希、draft CRUD、schema 校验、所有标量值类型、Link 基数和强制删除。
- Axum router 测试覆盖请求解析、图 projection 与状态映射。
- MySQL 8 集成测试放在 `wyse-ontology-mysql/tests/`，默认 workspace 测试中标记为忽略，并使用该 crate 的 `docker-compose.test.yml`。测试环境先运行 SQLx CLI migration。
- 端到端测试覆盖：创建 draft、schema CRUD、校验、发布/tag、读取图、Object 与 Link CRUD、表格分页、基数冲突、强制删除、历史 revision 读取、恢复 draft，以及拒绝不兼容发布。

## 参考资料

- Palantir：[Ontology 核心概念](https://www.palantir.com/docs/foundry/ontology/core-concepts)
- Palantir：[Ontology 设计最佳实践](https://www.palantir.com/docs/foundry/ontology/ontology-best-practices/)
- Microsoft：[Ontology Playground](https://github.com/microsoft/Ontology-Playground)
- MySQL：[JSON 数据类型](https://dev.mysql.com/doc/refman/8.4/en/json.html)
