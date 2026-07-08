use std::sync::Arc;

use serde_json::json;
use wyse_core::{CallId, ToolName, ToolSpec};
use wyse_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};
use wyse_tools::{
    ApplyPatchTool, BuiltinToolRegistry, EchoTool, ToolError, ToolInput, ToolRegistry,
};

async fn apply_patch_test_filesystem(name: &str) -> (Arc<LocalFilesystem>, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "wyse-tools-apply-patch-{name}-{}",
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
    (filesystem, root)
}

#[tokio::test]
async fn registered_echo_tool_can_be_called_through_registry() {
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()))
        .expect("echo tool should register");

    let output = registry
        .call(
            &ToolName::from("echo"),
            ToolInput::new(CallId::from("call-1"), json!({"message": "hello"})),
        )
        .await
        .expect("echo tool should run");

    assert_eq!(output.result, json!({"message": "hello"}));
    assert_eq!(
        registry.specs(),
        vec![
            ToolSpec::builder()
                .name("echo")
                .description("returns input arguments")
                .input_schema(json!({"type": "object"}))
                .build()
        ]
    );
}

#[test]
fn duplicate_registration_fails() {
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()))
        .expect("first registration should succeed");

    let error = registry
        .register(Arc::new(EchoTool::new()))
        .expect_err("duplicate registration should fail");

    assert!(matches!(
        error,
        ToolError::DuplicateTool { ref name } if name == &ToolName::from("echo")
    ));
}

#[tokio::test]
async fn missing_tool_returns_typed_error() {
    let registry = BuiltinToolRegistry::default();

    let error = registry
        .call(
            &ToolName::from("missing"),
            ToolInput::new(CallId::from("call-1"), json!({})),
        )
        .await
        .expect_err("missing tool should fail");

    assert!(matches!(
        error,
        ToolError::ToolNotFound { ref name } if name == &ToolName::from("missing")
    ));
}

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

#[tokio::test]
async fn apply_patch_tool_updates_existing_file_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("update").await;
    let path = VirtualPath::try_from("/src.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"one\ntwo\nthree\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(ApplyPatchTool::new(filesystem.clone())))
        .expect("apply patch tool should register");

    let output = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-update"),
                json!({
                    "operation": {
                        "type": "update_file",
                        "path": "/src.txt",
                        "diff": "@@\n one\n-two\n+deux\n three\n"
                    }
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(output.result["status"], "completed");
    assert_eq!(output.result["output"], "updated src.txt");
    assert_eq!(
        filesystem.read_file(&path).await.expect("read file"),
        b"one\ndeux\nthree\n"
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn apply_patch_tool_deletes_existing_file_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("delete").await;
    let path = VirtualPath::try_from("/old.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"old".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(ApplyPatchTool::new(filesystem.clone())))
        .expect("apply patch tool should register");

    let output = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-delete"),
                json!({
                    "operation": {
                        "type": "delete_file",
                        "path": "old.txt"
                    }
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(output.result["status"], "completed");
    assert_eq!(output.result["output"], "deleted old.txt");
    assert!(matches!(
        filesystem
            .read_file(&path)
            .await
            .expect_err("file should be gone"),
        wyse_filesystem::FilesystemError::NotFound { .. }
    ));

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn apply_patch_tool_returns_failed_status_for_patch_conflict() {
    let (filesystem, root) = apply_patch_test_filesystem("conflict").await;
    let path = VirtualPath::try_from("/src.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"one\ntwo\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(ApplyPatchTool::new(filesystem.clone())))
        .expect("apply patch tool should register");

    let output = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-conflict"),
                json!({
                    "operation": {
                        "type": "update_file",
                        "path": "src.txt",
                        "diff": "@@\n missing\n-two\n+deux\n"
                    }
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(output.result["status"], "failed");
    assert_eq!(
        output.result["output"],
        "patch context did not match src.txt"
    );
    assert_eq!(
        filesystem.read_file(&path).await.expect("read original"),
        b"one\ntwo\n"
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn apply_patch_tool_returns_failed_status_for_existing_create_target() {
    let (filesystem, root) = apply_patch_test_filesystem("exists").await;
    let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"original\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(ApplyPatchTool::new(filesystem.clone())))
        .expect("apply patch tool should register");

    let output = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-exists"),
                json!({
                    "operation": {
                        "type": "create_file",
                        "path": "notes.txt",
                        "diff": "@@\n+replacement\n"
                    }
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(output.result["status"], "failed");
    assert_eq!(
        output.result["output"],
        "file already exists at path 'notes.txt'"
    );
    assert_eq!(
        filesystem.read_file(&path).await.expect("read original"),
        b"original\n"
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn apply_patch_tool_rejects_invalid_relative_path() {
    let (filesystem, root) = apply_patch_test_filesystem("invalid-path").await;
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(ApplyPatchTool::new(filesystem)))
        .expect("apply patch tool should register");

    let error = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-invalid-path"),
                json!({
                    "operation": {
                        "type": "create_file",
                        "path": "../secret.txt",
                        "diff": "@@\n+secret\n"
                    }
                }),
            ),
        )
        .await
        .expect_err("invalid path should fail before patching");

    assert!(matches!(error, ToolError::InvalidPath { .. }));

    let _ = tokio::fs::remove_dir_all(root).await;
}
