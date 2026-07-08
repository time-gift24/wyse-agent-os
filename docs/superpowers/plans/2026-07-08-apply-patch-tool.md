# Apply Patch Tool Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Build a reusable `wyse-filesystem` apply-patch harness, expose it as an opt-in `wyse-tools` builtin `apply_patch` tool, and add crate-owned docker integration tests wired into GitHub Actions.

**Architecture:** `wyse-filesystem` owns patch operation types, V4A diff application, and all file mutation through `Filesystem`. `wyse-tools` owns JSON tool input/output, path normalization from OpenAI-style paths into `VirtualPath`, and explicit `Arc<dyn Filesystem>` injection. Container integration stays in `crates/wyse-tools` and `.github/workflows/rust.yml`.

**Tech Stack:** Rust 2024, Tokio, serde, serde_json, thiserror, async-trait, GitHub Actions, Docker Compose.

## Global Constraints

- Work in `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/apply-patch-tool-impl`.
- Do not implement multi-file transactions, backups, snapshots, approvals, rollback, shell execution, or direct host-path editing.
- Do not auto-register filesystem-mutating tools in `BuiltinToolRegistry`.
- Keep public paths as `VirtualPath` inside `wyse-filesystem`; normalize OpenAI-style relative paths only at the `wyse-tools` boundary.
- Recoverable patch failures return JSON `{ "status": "failed", "output": "..." }`, not `ToolError`.
- Do not expose host paths or file contents in errors, tool output, or tracing.
- Use TDD: write each behavior test, run it and observe failure, then implement the minimal code.
- Before completion run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace --all-targets`, and `make -C crates/wyse-tools test-integration`.

---

## File Structure

- Create `crates/wyse-filesystem/src/apply_patch.rs`: public patch operation/result types, V4A diff application, and `apply_patch`.
- Modify `crates/wyse-filesystem/src/lib.rs`: export the apply-patch module and public types.
- Modify `crates/wyse-tools/Cargo.toml`: add `wyse-filesystem` dependency.
- Create `crates/wyse-tools/src/builtin/apply_patch.rs`: builtin tool implementation and path normalization.
- Modify `crates/wyse-tools/src/builtin/mod.rs`: declare and re-export `ApplyPatchTool`.
- Modify `crates/wyse-tools/src/lib.rs`: re-export `ApplyPatchTool`.
- Modify `crates/wyse-tools/src/error.rs`: add typed malformed-input error for tool deserialization failures.
- Modify `crates/wyse-tools/tests/builtin_registry.rs`: registry-level tests for `ApplyPatchTool`.
- Create `crates/wyse-tools/tests/apply_patch_docker.rs`: ignored integration test for real `LocalFilesystem` sandbox behavior.
- Create `crates/wyse-tools/docker-compose.test.yml`: crate-owned test stack placeholder.
- Create `crates/wyse-tools/Makefile`: local integration command matching existing crate pattern.
- Modify `.github/workflows/rust.yml`: add `wyse-tools` integration setup, test, and cleanup.
- Modify `crates/wyse-filesystem/AGENTS.md`: archive final apply-patch filesystem convention.
- Create `crates/wyse-tools/AGENTS.md`: archive final tool convention.

---

### Task 1: Filesystem Patch Harness

**Files:**
- Create: `crates/wyse-filesystem/src/apply_patch.rs`
- Modify: `crates/wyse-filesystem/src/lib.rs`

**Interfaces:**
- Consumes: `Filesystem`, `FilesystemError`, `VirtualPath`.
- Produces:
  - `pub enum ApplyPatchOperationKind { CreateFile, UpdateFile, DeleteFile }`
  - `pub struct ApplyPatchOperation { pub kind: ApplyPatchOperationKind, pub path: VirtualPath, pub diff: Option<String> }`
  - `pub enum ApplyPatchStatus { Completed, Failed }`
  - `pub struct ApplyPatchOutput { pub status: ApplyPatchStatus, pub output: String }`
  - `pub enum ApplyPatchError`
  - `pub async fn apply_patch(filesystem: &dyn Filesystem, operation: &ApplyPatchOperation) -> Result<ApplyPatchOutput, ApplyPatchError>`

- [x] **Step 1: Write failing tests for create and update**

Add `crates/wyse-filesystem/src/apply_patch.rs` with type stubs and tests at the end. Keep implementations intentionally incomplete so tests fail for missing behavior:

```rust
//! Apply-patch operations for virtual filesystems.

use thiserror::Error;

use crate::{Filesystem, FilesystemError, VirtualPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplyPatchOperationKind {
    CreateFile,
    UpdateFile,
    DeleteFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApplyPatchOperation {
    pub kind: ApplyPatchOperationKind,
    pub path: VirtualPath,
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplyPatchStatus {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApplyPatchOutput {
    pub status: ApplyPatchStatus,
    pub output: String,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApplyPatchError {
    #[error("patch diff is required for {operation}")]
    MissingDiff { operation: &'static str },
    #[error("patch context did not match {path}")]
    ContextMismatch { path: VirtualPath },
    #[error("filesystem operation failed")]
    Filesystem {
        #[source]
        source: FilesystemError,
    },
}

pub async fn apply_patch(
    _filesystem: &dyn Filesystem,
    operation: &ApplyPatchOperation,
) -> Result<ApplyPatchOutput, ApplyPatchError> {
    Ok(ApplyPatchOutput {
        status: ApplyPatchStatus::Completed,
        output: format!("patched {}", operation.path),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("wyse-apply-patch-{name}-{}", std::process::id()))
    }

    async fn local_filesystem(name: &str) -> (LocalFilesystem, std::path::PathBuf) {
        let root = temp_root(name);
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.expect("create root");
        let filesystem = LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(4096),
        })
        .expect("filesystem is valid");
        (filesystem, root)
    }

    #[tokio::test]
    async fn create_file_applies_v4a_diff_to_empty_file() {
        let (filesystem, root) = local_filesystem("create").await;
        let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::CreateFile,
            path: path.clone(),
            diff: Some("@@\n+hello\n+world\n".to_owned()),
        };

        let output = apply_patch(&filesystem, &operation)
            .await
            .expect("patch should apply");

        assert_eq!(output.status, ApplyPatchStatus::Completed);
        assert_eq!(filesystem.read_file(&path).await.expect("read file"), b"hello\nworld\n");

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn update_file_applies_contextual_v4a_diff() {
        let (filesystem, root) = local_filesystem("update").await;
        let path = VirtualPath::try_from("/src.txt").expect("path is valid");
        filesystem
            .write_file(&path, b"one\ntwo\nthree\n".to_vec())
            .await
            .expect("seed file");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::UpdateFile,
            path: path.clone(),
            diff: Some("@@\n one\n-two\n+deux\n three\n".to_owned()),
        };

        let output = apply_patch(&filesystem, &operation)
            .await
            .expect("patch should apply");

        assert_eq!(output.status, ApplyPatchStatus::Completed);
        assert_eq!(
            filesystem.read_file(&path).await.expect("read file"),
            b"one\ndeux\nthree\n"
        );

        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
```

Add exports to `crates/wyse-filesystem/src/lib.rs`:

```rust
pub mod apply_patch;
pub use apply_patch::{
    ApplyPatchError, ApplyPatchOperation, ApplyPatchOperationKind, ApplyPatchOutput,
    ApplyPatchStatus, apply_patch,
};
```

- [x] **Step 2: Run tests to verify red**

Run: `cargo test -p wyse-filesystem apply_patch -- --nocapture`

Expected: tests compile but `create_file_applies_v4a_diff_to_empty_file` and `update_file_applies_contextual_v4a_diff` fail because the file contents are not changed.

- [x] **Step 3: Implement minimal create/update diff application**

Replace the stub with logic that:

- Requires `diff` for create and update.
- Reads existing content for update.
- Converts bytes with `String::from_utf8_lossy`.
- Applies one V4A hunk by walking lines beginning with ` `, `-`, and `+`.
- Writes the resulting bytes through `Filesystem::write_file`.
- Returns `ApplyPatchOutput { status: Completed, output: "created <path>" | "updated <path>" }`.

- [x] **Step 4: Run tests to verify green**

Run: `cargo test -p wyse-filesystem apply_patch -- --nocapture`

Expected: both apply-patch tests pass.

- [x] **Step 5: Add failing tests for delete, missing diff, and context mismatch**

Add tests in `apply_patch.rs`:

```rust
#[tokio::test]
async fn delete_file_removes_target() {
    let (filesystem, root) = local_filesystem("delete").await;
    let path = VirtualPath::try_from("/old.txt").expect("path is valid");
    filesystem.write_file(&path, b"old".to_vec()).await.expect("seed file");
    let operation = ApplyPatchOperation {
        kind: ApplyPatchOperationKind::DeleteFile,
        path: path.clone(),
        diff: None,
    };

    let output = apply_patch(&filesystem, &operation)
        .await
        .expect("delete should apply");

    assert_eq!(output.status, ApplyPatchStatus::Completed);
    assert!(matches!(
        filesystem.read_file(&path).await.expect_err("file should be gone"),
        crate::FilesystemError::NotFound { .. }
    ));

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn update_file_rejects_context_mismatch_without_writing_content() {
    let (filesystem, root) = local_filesystem("conflict").await;
    let path = VirtualPath::try_from("/src.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"one\ntwo\n".to_vec())
        .await
        .expect("seed file");
    let operation = ApplyPatchOperation {
        kind: ApplyPatchOperationKind::UpdateFile,
        path: path.clone(),
        diff: Some("@@\n missing\n-two\n+deux\n".to_owned()),
    };

    let error = apply_patch(&filesystem, &operation)
        .await
        .expect_err("context should fail");

    assert!(matches!(error, ApplyPatchError::ContextMismatch { .. }));
    assert_eq!(
        filesystem.read_file(&path).await.expect("read original"),
        b"one\ntwo\n"
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn create_file_requires_diff() {
    let (filesystem, root) = local_filesystem("missing-diff").await;
    let operation = ApplyPatchOperation {
        kind: ApplyPatchOperationKind::CreateFile,
        path: VirtualPath::try_from("/new.txt").expect("path is valid"),
        diff: None,
    };

    let error = apply_patch(&filesystem, &operation)
        .await
        .expect_err("diff is required");

    assert!(matches!(error, ApplyPatchError::MissingDiff { .. }));

    let _ = tokio::fs::remove_dir_all(root).await;
}
```

- [x] **Step 6: Run tests to verify red**

Run: `cargo test -p wyse-filesystem apply_patch -- --nocapture`

Expected: delete and error-path tests fail until delete and error handling are implemented.

- [x] **Step 7: Implement delete and typed error paths**

Add delete handling with `Filesystem::remove_file`. Map `FilesystemError` to `ApplyPatchError::Filesystem { source }`. Add a helper such as `fn operation_name(kind: ApplyPatchOperationKind) -> &'static str`.

- [x] **Step 8: Run tests to verify green**

Run: `cargo test -p wyse-filesystem apply_patch -- --nocapture`

Expected: all apply-patch filesystem tests pass.

---

### Task 2: ApplyPatchTool Wrapper

**Files:**
- Modify: `crates/wyse-tools/Cargo.toml`
- Create: `crates/wyse-tools/src/builtin/apply_patch.rs`
- Modify: `crates/wyse-tools/src/builtin/mod.rs`
- Modify: `crates/wyse-tools/src/lib.rs`
- Modify: `crates/wyse-tools/src/error.rs`

**Interfaces:**
- Consumes: `wyse_filesystem::{apply_patch, ApplyPatchOperation, ApplyPatchOperationKind, ApplyPatchStatus, Filesystem, VirtualPath}`.
- Produces: `pub struct ApplyPatchTool` with `pub fn new(filesystem: Arc<dyn Filesystem>) -> Self`.

- [x] **Step 1: Add dependency and failing registry test**

In `crates/wyse-tools/Cargo.toml`, add:

```toml
wyse-filesystem = { path = "../wyse-filesystem" }
```

In `crates/wyse-tools/tests/builtin_registry.rs`, import and test the desired public API:

```rust
use wyse_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};
use wyse_tools::{ApplyPatchTool, BuiltinToolRegistry, EchoTool, ToolError, ToolInput, ToolRegistry};
```

Add helper:

```rust
async fn apply_patch_test_filesystem(name: &str) -> (Arc<LocalFilesystem>, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("wyse-tools-apply-patch-{name}-{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&root).await.expect("create root");
    let filesystem = Arc::new(
        LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(4096),
        })
        .expect("filesystem is valid"),
    );
    (filesystem, root)
}
```

Add test:

```rust
#[tokio::test]
async fn apply_patch_tool_can_create_file_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("create").await;
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(ApplyPatchTool::new(filesystem.clone())))
        .expect("apply patch tool should register");

    let output = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-apply-patch"),
                json!({
                    "operation": {
                        "type": "create_file",
                        "path": "notes.txt",
                        "diff": "@@\n+hello\n"
                    }
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(output.result["status"], "completed");
    assert_eq!(output.result["output"], "created notes.txt");
    let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
    assert_eq!(
        filesystem.read_file(&path).await.expect("read file"),
        b"hello\n"
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}
```

- [x] **Step 2: Run test to verify red**

Run: `cargo test -p wyse-tools apply_patch_tool_can_create_file_through_registry -- --nocapture`

Expected: compile fails because `ApplyPatchTool` is not defined or exported.

- [x] **Step 3: Implement `ApplyPatchTool` create path**

Create `crates/wyse-tools/src/builtin/apply_patch.rs`:

```rust
//! Builtin apply-patch tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use wyse_core::ToolSpec;
use wyse_filesystem::{
    ApplyPatchError, ApplyPatchOperation, ApplyPatchOperationKind, ApplyPatchStatus, Filesystem,
    VirtualPath, apply_patch,
};

use crate::{Tool, ToolError, ToolInput, ToolOutput};

pub struct ApplyPatchTool {
    filesystem: Arc<dyn Filesystem>,
    spec: ToolSpec,
}

impl ApplyPatchTool {
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self {
            filesystem,
            spec: ToolSpec::builder()
                .name("apply_patch")
                .description("applies a create, update, or delete patch inside the virtual filesystem")
                .input_schema(json!({
                    "type": "object",
                    "required": ["operation"],
                    "properties": {
                        "operation": {
                            "type": "object",
                            "required": ["type", "path"],
                            "properties": {
                                "type": { "type": "string", "enum": ["create_file", "update_file", "delete_file"] },
                                "path": { "type": "string" },
                                "diff": { "type": "string" }
                            }
                        }
                    }
                }))
                .build(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApplyPatchInput {
    operation: RawOperation,
}

#[derive(Debug, Deserialize)]
struct RawOperation {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    diff: Option<String>,
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let raw: ApplyPatchInput = serde_json::from_value(input.arguments)
            .map_err(|source| ToolError::InvalidInput { source })?;
        let operation = operation_from_raw(raw.operation)?;
        let display_path = operation.path.as_str().trim_start_matches('/').to_owned();
        let result = match apply_patch(self.filesystem.as_ref(), &operation).await {
            Ok(output) => output,
            Err(error) => failed_output(error),
        };
        Ok(ToolOutput::new(json!({
            "status": status_text(result.status),
            "output": normalize_output(&result.output, &display_path),
        })))
    }
}

fn operation_from_raw(raw: RawOperation) -> Result<ApplyPatchOperation, ToolError> {
    let kind = match raw.kind.as_str() {
        "create_file" => ApplyPatchOperationKind::CreateFile,
        "update_file" => ApplyPatchOperationKind::UpdateFile,
        "delete_file" => ApplyPatchOperationKind::DeleteFile,
        _ => {
            return Err(ToolError::InvalidOperation {
                operation: raw.kind,
            });
        }
    };
    let path = normalize_path(&raw.path)?;
    Ok(ApplyPatchOperation {
        kind,
        path,
        diff: raw.diff,
    })
}
```

Add `ToolError` variants:

```rust
InvalidInput {
    #[source]
    source: serde_json::Error,
},
InvalidOperation {
    operation: String,
},
InvalidPath {
    path: String,
    #[source]
    source: wyse_filesystem::VirtualPathError,
},
```

Wire module exports in `builtin/mod.rs` and `lib.rs`.

- [x] **Step 4: Run test to verify green**

Run: `cargo test -p wyse-tools apply_patch_tool_can_create_file_through_registry -- --nocapture`

Expected: test passes.

- [x] **Step 5: Add failing tests for update, delete, failed status, and invalid path**

Add tests to `builtin_registry.rs`:

- `apply_patch_tool_updates_existing_file_through_registry`
- `apply_patch_tool_deletes_existing_file_through_registry`
- `apply_patch_tool_returns_failed_status_for_patch_conflict`
- `apply_patch_tool_rejects_invalid_relative_path`

Use JSON input matching the design and assert completed/failed status plus resulting filesystem state.

- [x] **Step 6: Run tests to verify red**

Run: `cargo test -p wyse-tools apply_patch_tool -- --nocapture`

Expected: at least failed-status or invalid-path behavior fails until mapping helpers are finished.

- [x] **Step 7: Finish path normalization and failed output mapping**

Implement helpers in `apply_patch.rs`:

- `normalize_path(path: &str) -> Result<VirtualPath, ToolError>`
- `status_text(status: ApplyPatchStatus) -> &'static str`
- `failed_output(error: ApplyPatchError) -> ApplyPatchOutput`
- `normalize_output(output: &str, display_path: &str) -> String`

Ensure `src/lib.rs` normalizes to `/src/lib.rs`, `/src/lib.rs` stays unchanged, and `../secret` is rejected.

- [x] **Step 8: Run tests to verify green**

Run: `cargo test -p wyse-tools apply_patch_tool -- --nocapture`

Expected: all apply-patch tool tests pass.

---

### Task 3: Docker Test Assets and CI

**Files:**
- Create: `crates/wyse-tools/docker-compose.test.yml`
- Create: `crates/wyse-tools/Makefile`
- Create: `crates/wyse-tools/tests/apply_patch_docker.rs`
- Modify: `.github/workflows/rust.yml`

**Interfaces:**
- Consumes: `ApplyPatchTool`, `LocalFilesystem`.
- Produces: ignored integration test runnable by `make -C crates/wyse-tools test-integration`.

- [x] **Step 1: Add failing ignored docker integration test**

Create `crates/wyse-tools/tests/apply_patch_docker.rs`:

```rust
use std::sync::Arc;

use serde_json::json;
use wyse_core::CallId;
use wyse_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};
use wyse_tools::{ApplyPatchTool, Tool, ToolInput};

#[ignore = "requires wyse-tools-test compose stack"]
#[tokio::test]
async fn apply_patch_tool_edits_local_sandbox_with_test_stack_running() {
    let root = std::env::temp_dir().join(format!(
        "wyse-tools-docker-apply-patch-{}",
        std::process::id()
    ));
    let _ = tokio::fs::remove_dir_all(&root).await;
    tokio::fs::create_dir_all(&root).await.expect("create root");
    let filesystem = Arc::new(
        LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(4096),
        })
        .expect("filesystem is valid"),
    );
    let tool = ApplyPatchTool::new(filesystem.clone());

    let create = tool
        .call(ToolInput::new(
            CallId::from("call-create"),
            json!({
                "operation": {
                    "type": "create_file",
                    "path": "docker.txt",
                    "diff": "@@\n+alpha\n+beta\n"
                }
            }),
        ))
        .await
        .expect("create call should run");
    assert_eq!(create.result["status"], "completed");

    let update = tool
        .call(ToolInput::new(
            CallId::from("call-update"),
            json!({
                "operation": {
                    "type": "update_file",
                    "path": "docker.txt",
                    "diff": "@@\n alpha\n-beta\n+gamma\n"
                }
            }),
        ))
        .await
        .expect("update call should run");
    assert_eq!(update.result["status"], "completed");

    let path = VirtualPath::try_from("/docker.txt").expect("path is valid");
    assert_eq!(
        filesystem.read_file(&path).await.expect("read file"),
        b"alpha\ngamma\n"
    );

    let failed = tool
        .call(ToolInput::new(
            CallId::from("call-failed"),
            json!({
                "operation": {
                    "type": "update_file",
                    "path": "../secret.txt",
                    "diff": "@@\n+nope\n"
                }
            }),
        ))
        .await
        .expect_err("invalid path should be a tool error");
    assert!(failed.to_string().contains("invalid path"));

    let _ = tokio::fs::remove_dir_all(root).await;
}
```

- [x] **Step 2: Run test to verify red or compile gap**

Run: `cargo test -p wyse-tools --test apply_patch_docker -- --ignored --nocapture`

Expected: fails until `ApplyPatchTool` is imported/exported correctly and helper behavior is complete.

- [x] **Step 3: Add compose and Makefile**

Create `crates/wyse-tools/docker-compose.test.yml`:

```yaml
name: wyse-tools-test

services:
  filesystem-sandbox:
    image: alpine:3.20
    command: ["sh", "-c", "mkdir -p /workspace && tail -f /dev/null"]
    volumes:
      - wyse-tools-sandbox:/workspace

volumes:
  wyse-tools-sandbox:
```

Create `crates/wyse-tools/Makefile`:

```make
COMPOSE ?= podman compose
TEST_COMPOSE_FILE := docker-compose.test.yml

.PHONY: test-integration test-up test-down

test-integration:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d
	cargo test -p wyse-tools --test apply_patch_docker -- --ignored --nocapture; \
	code=$$?; \
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v; \
	exit $$code

test-up:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) up -d

test-down:
	$(COMPOSE) -f $(TEST_COMPOSE_FILE) down -v
```

- [x] **Step 4: Update GitHub Actions**

In `.github/workflows/rust.yml`, add these steps after the `wyse-infra` cleanup step or split the `wyse-tools` stack with its own start/run/stop sequence:

```yaml
      - name: Start wyse-tools test stack
        if: steps.workspace.outputs.present == 'true'
        run: docker compose -f crates/wyse-tools/docker-compose.test.yml up -d

      - name: Run wyse-tools integration tests
        if: steps.workspace.outputs.present == 'true'
        run: cargo test -p wyse-tools --test apply_patch_docker -- --ignored --nocapture

      - name: Stop wyse-tools test stack
        if: always() && steps.workspace.outputs.present == 'true'
        run: docker compose -f crates/wyse-tools/docker-compose.test.yml down -v
```

- [x] **Step 5: Run docker integration**

Run: `make -C crates/wyse-tools test-integration`

Expected: compose stack starts, ignored integration test passes, stack is cleaned up.

---

### Task 4: Crate AGENTS Documentation

**Files:**
- Modify: `crates/wyse-filesystem/AGENTS.md`
- Create: `crates/wyse-tools/AGENTS.md`

**Interfaces:**
- Consumes: final implementation conventions.
- Produces: durable crate-local instructions.

- [x] **Step 1: Update filesystem crate instructions**

Modify `crates/wyse-filesystem/AGENTS.md` so the design rules say:

```markdown
- Apply-patch support is a concrete shared filesystem capability; it must keep all paths virtual and all reads/writes behind the `Filesystem` trait.
- Apply-patch errors and output must not expose host paths or file contents.
```

Also revise the old "Do not add apply_patch..." line to remove `apply_patch` from the deferred list.

- [x] **Step 2: Add tools crate instructions**

Create `crates/wyse-tools/AGENTS.md`:

```markdown
# wyse-tools AGENTS.md

## Scope

`wyse-tools` owns runtime tool traits, builtin tool wrappers, and tool registry behavior.

## Design Rules

- Tool names are provider-visible identities.
- Filesystem-mutating builtin tools require explicit filesystem injection.
- Do not auto-register filesystem-mutating tools in `BuiltinToolRegistry`.
- Recoverable tool-domain failures should return structured tool output when the caller can act on them.
- Keep concrete builtin implementations separate from registry code.
- Do not add remote tool adapters, MCP adapters, shell tools, or approval flows until a concrete caller needs them.
```

- [x] **Step 3: Verify docs are tracked**

Run: `git status --short crates/wyse-filesystem/AGENTS.md crates/wyse-tools/AGENTS.md`

Expected: both files are modified or added.

---

### Task 5: Final Verification and Commits

**Files:**
- All files changed by Tasks 1-4.

**Interfaces:**
- Consumes: completed implementation.
- Produces: verified branch ready for finishing flow.

- [x] **Step 1: Format**

Run: `cargo fmt --all`

Expected: exits 0.

- [x] **Step 2: Check formatting**

Run: `cargo fmt --all -- --check`

Expected: exits 0.

- [x] **Step 3: Clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`

Expected: exits 0.

- [x] **Step 4: Workspace tests**

Run: `cargo test --workspace --all-targets`

Expected: exits 0.

- [x] **Step 5: Docker integration**

Run: `make -C crates/wyse-tools test-integration`

Expected: exits 0 and compose cleanup runs.

- [x] **Step 6: Review diff**

Run: `git diff --stat && git diff --check`

Expected: no whitespace errors; diff only covers planned files.

- [x] **Step 7: Commit implementation**

Run:

```sh
git add \
  .github/workflows/rust.yml \
  crates/wyse-filesystem/AGENTS.md \
  crates/wyse-filesystem/src/apply_patch.rs \
  crates/wyse-filesystem/src/lib.rs \
  crates/wyse-tools/AGENTS.md \
  crates/wyse-tools/Cargo.toml \
  crates/wyse-tools/Makefile \
  crates/wyse-tools/docker-compose.test.yml \
  crates/wyse-tools/src/builtin/apply_patch.rs \
  crates/wyse-tools/src/builtin/mod.rs \
  crates/wyse-tools/src/error.rs \
  crates/wyse-tools/src/lib.rs \
  crates/wyse-tools/tests/apply_patch_docker.rs \
  crates/wyse-tools/tests/builtin_registry.rs \
  docs/superpowers/plans/2026-07-08-apply-patch-tool.md
git commit -m "feat: add apply patch tool"
```

Expected: commit succeeds on branch `codex/apply-patch-tool-impl`.
