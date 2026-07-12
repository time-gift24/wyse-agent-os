# Ontology Service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现一个以 MySQL 8 持久化发布 schema、以虚拟文件系统管理 draft、通过 Axum 暴露类型图和 Object/Link CRUD 的通用 Ontology Service。

**Architecture:** `wyse-ontology` 放置领域模型、schema 校验、draft CAS、发布和实例用例；`wyse-ontology-mysql` 是唯一 SQLx/MySQL 存储实现；`wyse-ontology-api` 只导出 Axum `router()` 与 DTO。draft 使用现有 `Filesystem::get/put` CAS，发布 revision 使用规范 schema JSON 的 SHA-256。

**Tech Stack:** Rust 2024、Tokio、Serde/serde_json、`sha2` 0.10.9、SQLx 0.8.6（MySQL）、MySQL 8、Axum 0.8.9、现有 `wyse-filesystem` CAS。

## 全局约束

- Rust edition 为 `2024`，最低 Rust 版本为 `1.88`；所有公共 ID 都是 UUID newtype，revision 是 64 位小写 SHA-256 hash。
- MySQL 固定版本 `8`；只用 SQLx CLI 手动运行 migration，应用代码绝不调用 `sqlx::migrate!`、建表或改表。
- 执行前必须用 `superpowers:using-git-worktrees` 创建非 `main` 的 `codex/ontology-service` worktree。
- 只管理一个逻辑 Ontology；不实现 memory、租户、workspace、权限、业务主键、SharedProperty、InterfaceType 或 ActionType。
- draft 固定在虚拟目录 `/ontology/drafts/`；HTTP 永不接收或返回主机/虚拟文件路径。
- 仅支持 `string`、`integer`、`number`、`boolean`、`datetime`、`json`；Object 值是 JSON，Link 只有类型和两端 Object。
- `online` 是保留 tag；实例写入同时满足请求 schema reference 与 `online` revision。
- 库 crate 使用 `thiserror`、`tracing` 和 `Result<T, OntologyError>`；生产代码不使用 `unwrap()`。
- MySQL 集成测试必须标记 `#[ignore]`，在 crate 自己的 `docker-compose.test.yml` 中使用 MySQL 8，并由 crate Makefile 通过 `sqlx migrate` 预建 schema。

---

## 文件结构

```text
Cargo.toml
crates/wyse-ontology/
  Cargo.toml
  AGENTS.md
  src/{lib,id,schema,value,draft,repository,service,graph,error}.rs
crates/wyse-ontology-mysql/
  Cargo.toml
  migrations/0001_ontology.sql
  src/{lib,repository,error}.rs
  tests/mysql_repository.rs
  docker-compose.test.yml
  Makefile
crates/wyse-ontology-api/
  Cargo.toml
  src/{lib,error,schema_routes,instance_routes}.rs
  tests/router.rs
docs/ontology-service-design.md
```

## 贯穿任务的公共接口

```rust
pub struct DraftName(String);
pub struct TagName(String);
pub struct RevisionId(String);
pub struct ObjectTypeId(Uuid);
pub struct PropertyTypeId(Uuid);
pub struct LinkTypeId(Uuid);
pub struct ObjectId(Uuid);
pub struct LinkId(Uuid);

pub enum SchemaRef {
    Draft(DraftName),
    Revision(RevisionId),
    Tag(TagName),
}

pub enum ValueType { String, Integer, Number, Boolean, Datetime, Json }
pub enum Cardinality { OneToOne, OneToMany, ManyToOne, ManyToMany }

pub struct ObjectRecord {
    pub id: ObjectId,
    pub object_type_id: ObjectTypeId,
    pub values: serde_json::Map<String, serde_json::Value>,
    pub version: u64,
}

pub struct LinkRecord {
    pub id: LinkId,
    pub link_type_id: LinkTypeId,
    pub source_object_id: ObjectId,
    pub target_object_id: ObjectId,
    pub version: u64,
}

pub struct PublishedRevision {
    pub id: RevisionId,
    pub schema: SchemaDocument,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct NewObjectRecord {
    pub object_type_id: ObjectTypeId,
    pub values: serde_json::Map<String, serde_json::Value>,
}

pub struct NewLinkRecord {
    pub link_type_id: LinkTypeId,
    pub source_object_id: ObjectId,
    pub target_object_id: ObjectId,
}

pub struct Page<T> {
    pub items: Vec<T>,
    pub next_after: Option<String>,
}

impl TagName {
    pub fn online() -> Self { Self("online".to_owned()) }
}
```

### Task 0: 创建隔离 worktree 并提交设计文档

**Files:**
- Add: `docs/ontology-service-design.md` 到新 worktree。
- Keep uncommitted: `docs/superpowers/plans/2026-07-11-ontology-service.md`，它是过程文档。

**Interfaces:**
- Consumes: 已确认的设计文档。
- Produces: 干净的 `codex/ontology-service` worktree。

- [ ] **Step 1: 创建 worktree**

调用 `superpowers:using-git-worktrees`。分支名使用 `codex/ontology-service`；若已存在则使用一个未占用的 `codex/ontology-service-*` 名称。

- [ ] **Step 2: 验证隔离状态**

Run:

```bash
git branch --show-current
git status --short
test -f docs/ontology-service-design.md
```

Expected: 分支以 `codex/` 开头，设计文档存在，当前用户无关改动没有被带入。

- [ ] **Step 3: 提交设计文档**

```bash
git add docs/ontology-service-design.md
git commit -m "docs: add ontology service design"
```

Expected: 只提交设计文档；不暂存 `skills-lock.json`、`.agents/`、`.claude/`、`.superpowers/` 或其它用户文件。

### Task 1: 建立领域 crate、ID 与 schema 静态校验

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/wyse-ontology/Cargo.toml`
- Create: `crates/wyse-ontology/src/lib.rs`
- Create: `crates/wyse-ontology/src/id.rs`
- Create: `crates/wyse-ontology/src/schema.rs`
- Create: `crates/wyse-ontology/src/error.rs`

**Interfaces:**
- Produces: `SchemaDocument::validate() -> Result<(), OntologyError>` 和全部公共 ID/newtype。

- [ ] **Step 1: 写失败测试**

在 `schema.rs` 末尾写重复名称、重复属性名、缺失 Link 端点和合法 schema 的测试：

```rust
#[test]
fn schema_rejects_a_link_with_a_missing_endpoint_type() {
    let schema = SchemaDocument {
        schema_version: 1,
        object_types: vec![person_type()],
        link_types: vec![LinkType::new(
            LinkTypeId::new(),
            "knows".to_owned(),
            person_type_id(),
            ObjectTypeId::new(),
            Cardinality::ManyToMany,
        )],
    };

    assert!(matches!(
        schema.validate(),
        Err(OntologyError::SchemaInvalid { .. })
    ));
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p wyse-ontology schema::tests::schema_rejects_a_link_with_a_missing_endpoint_type`
Expected: FAIL，`wyse-ontology` package 或 `SchemaDocument` 尚不存在。

- [ ] **Step 3: 写最小领域实现**

在 workspace 添加成员、路径依赖和以下新增共享依赖；不要引入 server、ORM、Git 或索引库：

```toml
axum = { version = "0.8.9", default-features = false, features = ["http1", "json", "query", "tokio"] }
sha2 = "0.10.9"
sqlx = { version = "0.8.6", default-features = false, features = ["chrono", "json", "mysql", "runtime-tokio-rustls", "uuid"] }
tower = { version = "0.5.2", default-features = false, features = ["util"] }
wyse-ontology = { path = "crates/wyse-ontology" }
```

每个 UUID ID 参照 `wyse-core` 现有 newtype 风格实现 `new`、`Default`、`Display`、`From<Uuid>`、`From<Id> for Uuid`、`FromStr` 和 `#[serde(transparent)]`。`RevisionId::try_from(String)` 只接收 64 位小写十六进制；`DraftName`/`TagName` 只接收 1–64 个 ASCII 字母、数字、`_`、`-`，且首字符为字母或数字。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyType {
    pub id: PropertyTypeId,
    pub name: String,
    pub description: String,
    pub value_type: ValueType,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkType {
    pub id: LinkTypeId,
    pub name: String,
    pub description: String,
    pub source_object_type_id: ObjectTypeId,
    pub target_object_type_id: ObjectTypeId,
    pub cardinality: Cardinality,
}
```

`validate()` 必须检查 schema version 为 `1`、各 UUID 唯一、ObjectType/LinkType 名称唯一、属性名称在所属 ObjectType 内唯一、Link 两端类型存在。错误汇入 `OntologyError::SchemaInvalid { diagnostics }`。

- [ ] **Step 4: 验证**

Run:

```bash
cargo test -p wyse-ontology
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/wyse-ontology
git commit -m "feat: add ontology schema model"
```

### Task 2: 实现规范 JSON、内容 hash 与 Filesystem CAS draft

**Files:**
- Modify: `crates/wyse-ontology/Cargo.toml`
- Modify: `crates/wyse-ontology/src/lib.rs`
- Create: `crates/wyse-ontology/src/draft.rs`
- Modify: `crates/wyse-ontology/src/error.rs`

**Interfaces:**
- Consumes: Task 1 的 `SchemaDocument`、`DraftName`、`RevisionId` 和 `Arc<dyn Filesystem>`。
- Produces: `FilesystemDraftStore::{create,load,list,replace,delete}` 与 `Draft { name, schema, digest }`。

- [ ] **Step 1: 写失败测试**

在 `draft.rs` 测试模块创建仅测试使用的 `MemoryCasFilesystem`；实现现有 `Filesystem::get/put`。测试规范字节稳定性、`Absent` 创建冲突与过期 digest：

```rust
#[tokio::test]
async fn replace_rejects_a_stale_digest() -> Result<(), OntologyError> {
    let store = test_store();
    let created = store
        .create(DraftName::try_from("main".to_owned())?, valid_schema())
        .await?;
    let changed = store
        .replace(&created.name, created.digest.clone(), changed_schema())
        .await?;
    let stale = store.replace(&created.name, created.digest, another_schema()).await;

    assert!(matches!(stale, Err(OntologyError::DraftConflict { .. })));
    assert_ne!(changed.digest, revision_id(&another_schema())?);
    Ok(())
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p wyse-ontology draft::tests::replace_rejects_a_stale_digest`
Expected: FAIL，`FilesystemDraftStore` 或 `revision_id` 未定义。

- [ ] **Step 3: 实现 draft store**

schema 没有 map，结构体字段与 `Vec` 顺序即为语义顺序，因此只使用 `serde_json::to_vec`，不增加 canonical JSON dependency：

```rust
pub fn canonical_schema_bytes(schema: &SchemaDocument) -> Result<Vec<u8>, OntologyError> {
    schema.validate()?;
    serde_json::to_vec(schema).map_err(OntologyError::EncodeSchema)
}

pub fn revision_id(schema: &SchemaDocument) -> Result<RevisionId, OntologyError> {
    use sha2::{Digest, Sha256};

    RevisionId::try_from(format!("{:x}", Sha256::digest(canonical_schema_bytes(schema)?)))
}
```

名称映射为 `/ontology/drafts/{name}.json`。`create` 用 `put(path, entry, CasExpectation::Absent)`；`load` 用 `get`；`replace` 先比较调用方 digest 与当前 bytes hash，再用 `put(path, entry, CasExpectation::Version(current.version))`；`delete` 同样先比较 digest。`VersionMismatch` 映射为 `DraftConflict`，`UnsupportedCas` 保持明确错误；不添加本地锁或降级到 `write_file`。

- [ ] **Step 4: 验证**

Run: `cargo test -p wyse-ontology draft::tests`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/wyse-ontology
git commit -m "feat: add ontology filesystem drafts"
```

### Task 3: 定义 repository contract、值校验与 schema 服务

**Files:**
- Create: `crates/wyse-ontology/src/repository.rs`
- Create: `crates/wyse-ontology/src/value.rs`
- Create: `crates/wyse-ontology/src/service.rs`
- Create: `crates/wyse-ontology/src/graph.rs`
- Modify: `crates/wyse-ontology/src/{lib,error}.rs`

**Interfaces:**
- Produces: `OntologyRepository`、`OntologyService`、`GraphProjection`、`validate_object_values()`。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn required_datetime_must_be_rfc3339_string() {
    let property = required_property("created_at", ValueType::Datetime);
    let values = Map::from_iter([("created_at".to_owned(), json!(42))]);

    assert!(matches!(
        validate_object_values(&[property], &values),
        Err(OntologyError::ValueInvalid { .. })
    ));
}

#[tokio::test]
async fn publishing_rejects_existing_values_invalid_for_the_draft() {
    let service = service_with_object(json!({"age":"old"}));

    assert!(matches!(
        service.publish(&DraftName::try_from("main".to_owned())?).await,
        Err(OntologyError::PublishInvalid { .. })
    ));
}
```

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p wyse-ontology value::tests service::tests`
Expected: FAIL，service/repository/value 校验未定义。

- [ ] **Step 3: 实现 repository trait 与纯领域逻辑**

trait 必须被 MySQL 实现和内存测试实现共同使用：

```rust
#[async_trait]
pub trait OntologyRepository: Send + Sync {
    async fn insert_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError>;
    async fn get_revision(&self, id: &RevisionId) -> Result<Option<PublishedRevision>, OntologyError>;
    async fn list_revisions(&self) -> Result<Vec<PublishedRevision>, OntologyError>;
    async fn put_tag(&self, name: &TagName, revision_id: &RevisionId) -> Result<(), OntologyError>;
    async fn get_tag(&self, name: &TagName) -> Result<Option<RevisionId>, OntologyError>;
    async fn delete_tag(&self, name: &TagName) -> Result<(), OntologyError>;
    async fn list_objects_for_schema_validation(&self) -> Result<Vec<ObjectRecord>, OntologyError>;
    async fn list_links_for_schema_validation(&self) -> Result<Vec<LinkRecord>, OntologyError>;
    async fn create_object(&self, object: NewObjectRecord) -> Result<ObjectRecord, OntologyError>;
    async fn get_object(&self, id: ObjectId) -> Result<Option<ObjectRecord>, OntologyError>;
    async fn page_objects(&self, type_id: ObjectTypeId, after: Option<ObjectId>, limit: u32) -> Result<Page<ObjectRecord>, OntologyError>;
    async fn replace_object(&self, object: ObjectRecord) -> Result<ObjectRecord, OntologyError>;
    async fn delete_object(&self, id: ObjectId, version: u64, force: bool) -> Result<(), OntologyError>;
    async fn create_link(&self, link: NewLinkRecord) -> Result<LinkRecord, OntologyError>;
    async fn get_link(&self, id: LinkId) -> Result<Option<LinkRecord>, OntologyError>;
    async fn page_links(&self, after: Option<LinkId>, limit: u32) -> Result<Page<LinkRecord>, OntologyError>;
    async fn replace_link(&self, link: LinkRecord) -> Result<LinkRecord, OntologyError>;
    async fn delete_link(&self, id: LinkId, version: u64) -> Result<(), OntologyError>;
    async fn links_for_cardinality(&self, type_id: LinkTypeId, source: ObjectId, target: ObjectId, excluding: Option<LinkId>) -> Result<Vec<LinkRecord>, OntologyError>;
}
```

`OntologyService::resolve_schema` 从 draft、revision 或 tag 取 schema。`publish` 依次加载 draft、`SchemaDocument::validate()`、读取全部实例、用 draft 校验每条 Object/Link，成功后插入 `PublishedRevision`。tag 只能指向存在 revision，`online` 禁止删除。

在该任务的测试模块中定义私有 `service_with_object() -> OntologyService`、`person_type()` 和内存 `OntologyRepository` fixture；它们只组装上面已定义的公开类型，不导出到 crate API。

`validate_object_values` 拒绝未知 property；`required` 必须存在；`string`、`integer`、`number`、`boolean` 使用 JSON 对应类型，`datetime` 用 `DateTime::parse_from_rfc3339`，`json` 接受任意 JSON。`graph.rs` 只输出 ObjectType nodes 与 LinkType edges，不保存位置、样式或实例。

- [ ] **Step 4: 验证**

Run:

```bash
cargo test -p wyse-ontology
cargo clippy -p wyse-ontology --all-targets -- -D warnings
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/wyse-ontology
git commit -m "feat: add ontology validation service"
```

### Task 4: 实现 SQLx/MySQL repository 与 CLI migration 资产

**Files:**
- Create: `crates/wyse-ontology-mysql/{Cargo.toml,src/lib.rs,src/repository.rs,src/error.rs}`
- Create: `crates/wyse-ontology-mysql/migrations/0001_ontology.sql`
- Create: `crates/wyse-ontology-mysql/{tests/mysql_repository.rs,docker-compose.test.yml,Makefile}`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: Task 3 的 `OntologyRepository`。
- Produces: `SqlxOntologyRepository::new(MySqlPool)`。

- [ ] **Step 1: 写失败 MySQL 8 集成测试**

```rust
#[tokio::test]
#[ignore = "requires MySQL 8 started by the crate Makefile"]
async fn repository_persists_revision_and_online_tag() -> Result<(), Box<dyn std::error::Error>> {
    let pool = MySqlPool::connect(&std::env::var("DATABASE_URL")?).await?;
    let repository = SqlxOntologyRepository::new(pool);
    let revision = published_revision();

    repository.insert_revision(revision.clone()).await?;
    repository.put_tag(&TagName::try_from("online".to_owned())?, &revision.id).await?;
    assert_eq!(
        repository.get_tag(&TagName::try_from("online".to_owned())?).await?,
        Some(revision.id)
    );
    Ok(())
}
```

- [ ] **Step 2: 确认 migration 前失败**

Run: `DATABASE_URL=mysql://ontology:ontology@127.0.0.1:33067/wyse_ontology cargo test -p wyse-ontology-mysql --test mysql_repository -- --ignored`
Expected: FAIL，package、连接或表不存在。

- [ ] **Step 3: 写静态 DDL 和 repository**

`0001_ontology.sql` 只创建 `ontology_revisions`、`ontology_tags`、`objects`、`links`。UUID 使用 `CHAR(36) CHARACTER SET ascii`；revision 使用 `CHAR(64) CHARACTER SET ascii`；所有表使用 InnoDB/utf8mb4。`objects` 包含 `values_json JSON`、`version BIGINT UNSIGNED` 和 `object_type_id` 索引；`links` 有 source/target 到 `objects(id)` 的外键，以及 type/source/target 索引。

```sql
UPDATE objects
SET values_json = ?, version = version + 1, updated_at = UTC_TIMESTAMP(6)
WHERE id = ? AND version = ?;
```

使用 `sqlx::query()` 与 bind 参数，不使用 `query!` 宏或编译期数据库连接。更新受影响行数为零时先查询：缺失映射 `ObjectMissing`，存在映射 `VersionConflict`。force delete 必须在一个 transaction 内先删除两端匹配的 Link，再带版本删除 Object；非 force delete 直接删除并把外键错误映射 `ObjectReferenced`。

`docker-compose.test.yml` 使用 `mysql:8`、项目名 `wyse-ontology-mysql-test`、端口 `33067:3306`、数据库/用户/密码 `wyse_ontology`/`ontology`/`ontology` 和 `mysqladmin ping` healthcheck。Makefile 使用 `COMPOSE ?= podman compose`，顺序运行：

```make
$(COMPOSE) -f docker-compose.test.yml up -d --wait
DATABASE_URL=mysql://ontology:ontology@127.0.0.1:33067/wyse_ontology sqlx migrate run --source migrations
DATABASE_URL=mysql://ontology:ontology@127.0.0.1:33067/wyse_ontology cargo test -p wyse-ontology-mysql --test mysql_repository -- --ignored
```

退出时执行 `down -v`。

- [ ] **Step 4: 验证**

Run: `make -C crates/wyse-ontology-mysql test-integration COMPOSE="podman compose"`
Expected: PASS，CLI migration 和 ignored MySQL 8 测试均成功，volume 已清理。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/wyse-ontology-mysql
git commit -m "feat: add mysql ontology repository"
```

### Task 5: 完成 Object/Link CRUD、双 schema 写校验与基数

**Files:**
- Modify: `crates/wyse-ontology/src/{service,repository,error}.rs`

**Interfaces:**
- Consumes: Task 2–4。
- Produces: `create/replace/delete/page_object` 与 `create/replace/delete/page_link` 用例。

- [ ] **Step 1: 写失败测试**

```rust
#[tokio::test]
async fn draft_write_cannot_break_the_online_schema() {
    let service = service_with_online_integer_age_and_draft_string_age();
    let request = CreateObject {
        schema_ref: SchemaRef::Draft(DraftName::try_from("experiment".to_owned()).unwrap()),
        object_type_id: person_type_id(),
        values: Map::from_iter([("age".to_owned(), json!("old"))]),
    };

    assert!(matches!(
        service.create_object(request).await,
        Err(OntologyError::ValueInvalid { .. })
    ));
}
```

加入 `force=false` 删除有 Link 的 Object 返回 `ObjectReferenced`、`force=true` 同时删除 Object/Link、`ManyToOne` 拒绝同一 source 的第二条 Link 的测试。

测试模块定义私有 `service_with_online_integer_age_and_draft_string_age() -> OntologyService` fixture；它创建同一 ObjectType UUID 的 `online` integer schema 与 `experiment` string draft。

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p wyse-ontology service::tests::draft_write_cannot_break_the_online_schema`
Expected: FAIL，实例用例尚未完整。

- [ ] **Step 3: 实现 post-state 校验**

每个写操作都按以下顺序；API 不得直接访问 repository：

```rust
let requested = self.resolve_schema(&request.schema_ref).await?;
let online = self.resolve_schema(&SchemaRef::Tag(TagName::online())).await?;
validate_object_in_schema(&requested, &candidate)?;
validate_object_in_schema(&online, &candidate)?;
self.repository.replace_object(candidate).await
```

Link 创建/更新还需针对 requested 和 online schema 各自检查 LinkType 端点类型和 cardinality。基数规则固定为：

```rust
match cardinality {
    Cardinality::OneToOne => source_count == 0 && target_count == 0,
    Cardinality::OneToMany => target_count == 0,
    Cardinality::ManyToOne => source_count == 0,
    Cardinality::ManyToMany => true,
}
```

`PATCH /objects` 使用完整 `values` 替换，`PATCH /links` 使用完整新端点替换；不实现 JSON Patch、Link 属性、动态过滤或字段级 mutation。分页 limit 只接受 `1..=100`，cursor 为 UUID。

- [ ] **Step 4: 验证**

Run:

```bash
cargo test -p wyse-ontology service::tests
cargo clippy -p wyse-ontology --all-targets -- -D warnings
```

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add crates/wyse-ontology
git commit -m "feat: add ontology instance operations"
```

### Task 6: 实现 draft/revision/tag/类型图 Axum API

**Files:**
- Create: `crates/wyse-ontology-api/{Cargo.toml,src/lib.rs,src/error.rs,src/schema_routes.rs,tests/router.rs}`
- Modify: `Cargo.toml`

**Interfaces:**
- Consumes: `Arc<OntologyService>`。
- Produces: `pub fn router(service: Arc<OntologyService>) -> Router` 与 schema HTTP routes。

- [ ] **Step 1: 写失败路由测试**

```rust
#[tokio::test]
async fn graph_route_returns_schema_nodes_and_edges() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_online_schema());
    let response = app
        .oneshot(Request::builder()
            .uri("/v1/ontology/graph?tag=online")
            .body(Body::empty())?)
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let body: GraphResponse = decode_json(response).await?;
    assert_eq!(body.nodes.len(), 2);
    assert_eq!(body.edges.len(), 1);
    Ok(())
}
```

同一文件测试：创建 draft、过期 `If-Match` 返回 `412`、`POST /validate` 对静态 schema 错误返回 `422`、删除 `online` tag 返回 `409`。

在测试模块定义私有 `test_service_with_online_schema() -> Arc<OntologyService>`、`decode_json()` 和内存 CAS/repository fixture；它们不属于生产 API。

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p wyse-ontology-api --test router graph_route_returns_schema_nodes_and_edges`
Expected: FAIL，API crate 或 `router()` 未定义。

- [ ] **Step 3: 写最小 router 和 schema routes**

```rust
#[derive(Clone)]
pub struct AppState {
    service: Arc<OntologyService>,
}

pub fn router(service: Arc<OntologyService>) -> Router {
    Router::new()
        .nest("/v1/ontology", schema_routes())
        .with_state(AppState { service })
}
```

实现：

```text
POST /drafts; GET /drafts; GET/DELETE /drafts/{name}; POST /drafts/{name}/validate
POST/PATCH/DELETE /drafts/{name}/object-types[/{id}]
POST/PATCH/DELETE /drafts/{name}/object-types/{id}/properties[/{property_id}]
POST/PATCH/DELETE /drafts/{name}/link-types[/{id}]
POST /drafts/{name}/publish; GET /revisions; GET /revisions/{id}
GET/PUT/DELETE /tags/{name}; GET /graph
```

`/graph` query DTO 有 `draft`/`revision`/`tag` 三个 `Option`；只接受恰好一个。缺失资源映射 `404`，draft/实例版本冲突 `412`，引用/基数/重复 `409`，schema/value 诊断 `422`，其余 `500`。不创建 listener、不读环境变量、不加 deployment binary。

- [ ] **Step 4: 验证**

Run: `cargo test -p wyse-ontology-api --test router`
Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml crates/wyse-ontology-api
git commit -m "feat: add ontology schema api"
```

### Task 7: 实现 Object/Link Axum API 与端到端验证

**Files:**
- Create: `crates/wyse-ontology-api/src/instance_routes.rs`
- Modify: `crates/wyse-ontology-api/src/lib.rs`
- Modify: `crates/wyse-ontology-api/tests/router.rs`
- Create: `crates/wyse-ontology/AGENTS.md`

**Interfaces:**
- Consumes: Tasks 1–6。
- Produces: Object/Link 完整 REST CRUD、分页表格接口和可重复端到端证据。

- [ ] **Step 1: 写失败 API 测试**

```rust
#[tokio::test]
async fn object_list_is_paginated_and_force_delete_removes_incident_links() -> Result<(), Box<dyn std::error::Error>> {
    let app = router(test_service_with_two_people_and_link());
    let list_uri = format!(
        "/v1/objects?tag=online&object_type_id={}&limit=1",
        test_person_type_id(),
    );
    let delete_uri = format!(
        "/v1/objects/{}?tag=online&force=true&version=1",
        test_first_object_id(),
    );
    let page = app.clone()
        .oneshot(Request::builder()
            .uri(list_uri)
            .body(Body::empty())?)
        .await?;
    assert_eq!(page.status(), StatusCode::OK);

    let deleted = app
        .oneshot(Request::builder()
            .method("DELETE")
            .uri(delete_uri)
            .body(Body::empty())?)
        .await?;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    Ok(())
}
```

加入未强制删除为 `409`、过期 Object/Link version 为 `412`、违反 `OneToMany` 为 `409`、未知 Object property 为 `422` 的测试。

在测试模块定义私有 `test_service_with_two_people_and_link() -> Arc<OntologyService>`、`test_person_type_id() -> ObjectTypeId` 和 `test_first_object_id() -> ObjectId`；fixture 使用固定 UUID，保证 URI 与断言可重复。

- [ ] **Step 2: 确认测试失败**

Run: `cargo test -p wyse-ontology-api --test router object_list_is_paginated_and_force_delete_removes_incident_links`
Expected: FAIL，`instance_routes` 尚未注册。

- [ ] **Step 3: 实现实例路由**

```text
POST   /v1/objects                    { schema_ref, object_type_id, values }
GET    /v1/objects?draft|revision|tag&object_type_id&after&limit
GET    /v1/objects/{id}?draft|revision|tag
PATCH  /v1/objects/{id}               { schema_ref, version, values }
DELETE /v1/objects/{id}?draft|revision|tag&version&force
POST   /v1/links                      { schema_ref, link_type_id, source_object_id, target_object_id }
GET    /v1/links?draft|revision|tag&after&limit
GET    /v1/links/{id}?draft|revision|tag
PATCH  /v1/links/{id}                 { schema_ref, version, source_object_id, target_object_id }
DELETE /v1/links/{id}?draft|revision|tag&version
```

response 固定使用：

```rust
#[derive(Serialize)]
struct PageResponse<T> {
    items: Vec<T>,
    next_after: Option<String>,
}
```

handler 只解析 DTO、调用 Service、返回 DTO；不复制 schema、值类型或基数逻辑。

- [ ] **Step 4: 写并通过完整链路测试**

在同一 router 测试文件加入流程：创建 draft → 创建两个 ObjectType 与一个 `OneToMany` LinkType → validate → publish → `PUT /tags/online` → 取 graph → 创建两个 Object → 创建 Link → 分页取 Object → 基数冲突 → force delete → 读取 revision → 从 revision 创建新 draft。至少断言：

```rust
assert_eq!(published.status(), StatusCode::CREATED);
assert_eq!(graph.status(), StatusCode::OK);
assert_eq!(cardinality_conflict.status(), StatusCode::CONFLICT);
assert_eq!(forced_delete.status(), StatusCode::NO_CONTENT);
assert_eq!(restored_draft.status(), StatusCode::CREATED);
```

Run:

```bash
cargo test -p wyse-ontology-api
cargo test --workspace --all-targets
make -C crates/wyse-ontology-mysql test-integration COMPOSE="podman compose"
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: 全部 PASS。

- [ ] **Step 5: 归档约定并提交**

`crates/wyse-ontology/AGENTS.md` 只记录：draft 虚拟目录、revision SHA-256、`online` 双 schema 写校验、Object JSON/Link 无值、migration 仅由 CLI 执行。不要复制本计划或其它 superpowers 过程文档。

```bash
git add crates/wyse-ontology crates/wyse-ontology-api docs/ontology-service-design.md
git commit -m "test: verify ontology service flow"
git status --short
```

Expected: 只剩 `docs/superpowers/plans/` 过程文档未跟踪；提醒用户在合并前审阅新 crate 的 `AGENTS.md` 归档。

## 计划自检

| Spec 要求 | 任务 |
| --- | --- |
| 最小 schema、UUID、JSON Object 值 | 1、3 |
| 文件 draft、CAS、SHA-256 revision | 2 |
| 不可变 revision、tag、`online` | 3、4、6 |
| MySQL 8、SQLx CLI migration、静态表 | 4 |
| 共享实例、双 schema 校验、基数、force delete | 5 |
| 类型图、分页表格 | 3、6、7 |
| Axum REST/JSON 与 HTTP 错误 | 6、7 |
| 单元、路由、MySQL 8 与端到端测试 | 1–7 |
| crate AGENTS.md 归档 | 7 |

自检结果：所有确认需求都有任务；计划没有 Git、动态 DDL、本地锁、部署 binary、动态属性索引或额外领域原语。
