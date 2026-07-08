# AGENTS.md

## 项目目标

Wyse Agent OS 是一个 Rust-first 的 agent runtime 和工作流编排系统。实现时优先保持模块化、强类型、可观测，并默认安全。

## Rust 开发规范

编写、审查或重构 Rust 代码时，遵循本地 `rust-skills` 规范：`.agents/skills/rust-skills/SKILL.md`。规则优先级如下：

1. Ownership and borrowing
2. Error handling
3. Memory optimization
4. Unsafe code
5. API design
6. Async/await
7. Concurrency
8. Type safety
9. Testing
10. Observability

## Workspace 结构

- 使用 Cargo workspace，并按能力拆分小而清晰的 crates。
- 每个 crate 的职责要窄且明确。
- 模块按功能组织，不按泛泛的类型类别组织。
- trait 定义和具体实现必须分离：trait 文件只放接口、关联类型和必要的轻量 helper，具体实现放到按能力或后端命名的模块中。
- error 定义必须和 trait / implementation 分离；优先放在 crate 内的 `error.rs`，复杂领域可以使用独立的领域 error 模块。
- 保持 `main.rs` 足够薄，可复用逻辑放到 `lib.rs`。
- 共享依赖版本通过 workspace dependency inheritance 管理。
- Cargo features 必须是 additive，不要让 feature 之间互相排斥或改变已有行为。
- 每个 crate 自己维护与自身相关的轻量测试资产，例如 `docker-compose.test.yml`、`Makefile` 和 `tests/`。

## API 设计

- 领域 ID 使用 newtype，例如 `RunId`、`AgentId`、`CallId`、`ModelId`。
- 避免 stringly typed API；能用 enum 或已校验 newtype 表达的，不要只用裸字符串。
- 公共类型在合适时实现常用 trait：`Debug`、`Clone`、`PartialEq`、`Eq`、`Hash`、`Serialize`、`Deserialize`。
- 实现转换时优先实现 `From<T>`，不要手写 `Into<T>`。
- 可失败的解析和转换使用 `TryFrom`、`FromStr`。
- 未来可能增加字段或变体的公共 struct/enum 使用 `#[non_exhaustive]`。
- 复杂对象构造使用 builder，并给 builder 方法加 `#[must_use]`。

## 克制设计

- 默认选择能工作的最小设计，先解决当前明确需求。
- 禁止为了“以后可能需要”提前增加 wrapper 层、adapter 层、facade 层、manager 层或 snapshot 机制。
- 禁止在没有明确收益和讨论结论的情况下引入设计模式，例如 factory、strategy、observer、repository、service locator 等。
- 一个 trait 至少要有真实的多实现需求；只有一个实现时，优先使用具体类型。
- 一个配置项至少要有真实使用场景；不会被用户或调用方改变的值不要配置化。
- 一个 abstraction 必须减少真实重复、隔离真实外部边界，或编码重要不变量；否则不要添加。
- 优先复用 Rust 标准库、已有 crate 内部函数和已经引入的依赖，不为几行代码新增依赖。
- 需要明显扩展点时，先在 TODO 或注释里记录边界，等需求出现后再实现。
- 如果认为必须引入额外层次或设计模式，先在回复中说明原因、替代方案和成本，经过讨论后再落代码。

## 错误处理

- library crates 使用 `thiserror` 定义类型化错误。
- 使用 `thiserror::Error` derive；不要手写字符串型错误，也不要把 error enum 混在 trait 或具体实现文件里。
- application binaries 可以在顶层使用 `anyhow`。
- 可恢复失败返回 `Result<T, E>`。
- 生产代码不要使用 `unwrap()`。
- `expect()` 只用于表示程序员错误的不变量。
- 用 `#[source]` 或 `From` 转换保留错误来源链。
- 错误消息使用小写开头，不加句号。
- 可失败的公共函数需要在文档里写 `# Errors`。

## Ownership 和内存

- 优先借用，避免不必要的 clone。
- 参数优先接收 `&str` 而不是 `&String`，接收 `&[T]` 而不是 `&Vec<T>`。
- 跨线程共享所有权使用 `Arc<T>`。
- 如果 enum 的大变体会明显增大整体尺寸，考虑 boxing。
- 已知容量时使用 `with_capacity` 预分配。
- 热路径中尽量复用 collection，避免重复分配。
- 热路径中避免不必要的 `format!`，能直接写入或使用字面量就直接使用。

## Async 和并发

- async runtime 使用 Tokio。
- 不要在 `.await` 期间持有 `Mutex` 或 `RwLock` guard。
- 队列和背压使用 bounded channels。
- 运行取消和优雅关闭使用 `CancellationToken`。
- 动态任务集合使用 `JoinSet` 管理。
- CPU-heavy 或 blocking 工作使用 `spawn_blocking`。
- `tokio::select!` 分支要满足 cancellation-safe。
- trait API 合适时优先使用原生 `async fn`。

## Unsafe 代码

- 除非有清晰且可衡量的必要性，否则不要使用 `unsafe`。
- 每个 `unsafe` block 前必须有 `// SAFETY:` 注释说明不变量。
- 每个 `unsafe fn` 必须有 `# Safety` 文档。
- `unsafe` 作用域越小越好。
- 不要使用 `mem::uninitialized()`，也不要对有有效性约束的类型使用无效的 `mem::zeroed()`。

## 序列化

- serde 命名规则要匹配外部 payload，通常使用 `#[serde(rename_all = "snake_case")]` 或协议要求的 casing。
- 向后兼容的可选字段使用 `#[serde(default)]`。
- 空 optional 字段使用 `skip_serializing_if`。
- 边界数据尽量在反序列化时完成校验。
- 对严格配置格式，使用拒绝未知字段的策略，避免配置拼写错误被静默忽略。

## 可观测性

- 使用 `tracing` 做结构化日志和 spans。
- library 只通过 tracing/log facade 发出事件，不安装全局 subscriber。
- 不要记录 secret、token、原始凭据或敏感用户数据。
- 上下文信息放到 structured fields，不要拼进字符串里。
- 错误只在真正处理它的边界记录一次。

## 测试

- 单元测试放在 `#[cfg(test)] mod tests` 中。
- 跨 crate 集成测试放在 `tests/` 目录。
- 测试命名要描述被验证的行为。
- 测试结构保持 arrange、act、assert 清晰。
- 测试模块和测试函数不得放在文件头部；应放在被测生产代码之后，通常放在文件末尾。
- 禁止为了测试方便在生产 API 中添加函数；测试辅助逻辑应放在测试模块、`tests/` helper 或 fixture 中。
- async 测试使用 `#[tokio::test]`。
- agent、LLM、tool、MCP 相关测试使用 mock provider 和基于 trait 的依赖。
- parser、validator、graph scheduling、schema conversion 优先考虑 property tests。
- 需要真实外部依赖的集成测试放在对应 crate 的 `tests/` 目录，并默认标记 `#[ignore]`，避免普通 `cargo test --workspace --all-targets` 依赖容器。
- 每个 crate 的集成测试容器使用独立的 `docker-compose.test.yml`；compose project name 使用 crate 名加 `-test`，例如 `wyse-infra-test`。
- 本地集成测试优先通过 crate 内 `Makefile` 运行；默认使用 `podman compose`，需要 Docker 时用 `COMPOSE="docker compose"` 覆盖。
- CI 中普通单测和容器集成测试分成不同 job；集成测试 job 显式启动对应 crate 的 compose 测试栈，运行 ignored tests，最后 `down -v` 清理。

## Lint 和格式化

- 提交 Rust 变更前运行 `cargo fmt`。
- 有意义的 Rust 变更运行 `cargo clippy --workspace --all-targets`。
- workspace skeleton 创建后，在 workspace 层统一配置 lints。
- 先启用 correctness、suspicious、style、complexity、perf 相关 lints。
- 不要无理由 silence lint；确实需要时写简短原因。

## 文档

- 公共 API 使用 `///` 文档。
- crate 和 module 的意图使用 `//!` 模块级文档说明。
- 重要公共 API 尽量提供可运行示例。
- 相关类型之间使用 intra-doc links。
- 示例中避免 `unwrap()`，优先使用 `?`。
- 不要提交 `docs/superpowers/` 这类 superpower 过程文档；它们只用于临时协作。
- 最终设计和实现约定要简洁、明确地归档到相关 crate 的 `AGENTS.md` 中。
- 实现完成后、PR 合入前，必须提醒用户完成 crate `AGENTS.md` 归档。

## Git 工作流

- 开始实现前务必使用 `using-git-worktrees` 创建新的独立工作目录；不要在当前 checkout 里直接切分支或改动。
- 禁止直接在 `main` 分支 commit。
- 需要提交时先创建或切换到非 `main` 分支，默认分支名使用 `codex/` 前缀。

## 实现风格

- 优先写清晰、朴素、可维护的 Rust。
- 只有当抽象能消除真实重复或编码重要不变量时才引入抽象。
- 依赖保持明确且尽量少。
- 即使内部还在演进，公共 API 也要看起来稳定、克制。
- 保留用户在工作区中的改动，不要回滚无关文件。
