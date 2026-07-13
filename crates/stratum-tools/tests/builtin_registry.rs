use std::sync::Arc;

use serde_json::json;
use stratum_core::{CallId, DangerLevel, ToolKind, ToolName, ToolSpec};
use stratum_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};
use stratum_tools::{
    ApplyPatchTool, BuiltinToolRegistry, EchoTool, FileMetadataTool, ListDirTool,
    ReadFileLinesTool, SearchTextTool, ToolError, ToolInput, ToolPermissionMode, ToolRegistry,
};

async fn apply_patch_test_filesystem(name: &str) -> (Arc<LocalFilesystem>, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "stratum-tools-apply-patch-{name}-{}",
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
async fn read_file_lines_tool_returns_requested_line_range_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("read-lines").await;
    let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"alpha\nbeta\ngamma\ndelta\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ReadFileLinesTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("read file lines tool should register");

    let output = registry
        .call(
            &ToolName::from("read_file_lines"),
            ToolInput::new(
                CallId::from("call-read-lines"),
                json!({
                    "path": "notes.txt",
                    "start_line": 2,
                    "line_count": 2
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(
        output.result,
        json!({
            "path": "notes.txt",
            "start_line": 2,
            "end_line": 3,
            "total_lines": 4,
            "lines": [
                {"line_number": 2, "text": "beta"},
                {"line_number": 3, "text": "gamma"}
            ]
        })
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn read_file_lines_tool_returns_empty_lines_when_range_starts_after_end() {
    let (filesystem, root) = apply_patch_test_filesystem("read-lines-empty").await;
    let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"alpha\nbeta\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ReadFileLinesTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("read file lines tool should register");

    let output = registry
        .call(
            &ToolName::from("read_file_lines"),
            ToolInput::new(
                CallId::from("call-read-lines-empty"),
                json!({
                    "path": "notes.txt",
                    "start_line": 5,
                    "line_count": 2
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(
        output.result,
        json!({
            "path": "notes.txt",
            "start_line": 5,
            "end_line": null,
            "total_lines": 2,
            "lines": []
        })
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn read_file_lines_tool_rejects_invalid_relative_path() {
    let (filesystem, root) = apply_patch_test_filesystem("read-lines-invalid-path").await;
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ReadFileLinesTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("read file lines tool should register");

    let error = registry
        .call(
            &ToolName::from("read_file_lines"),
            ToolInput::new(
                CallId::from("call-read-lines-invalid-path"),
                json!({
                    "path": "../secret.txt",
                    "start_line": 1,
                    "line_count": 1
                }),
            ),
        )
        .await
        .expect_err("invalid path should fail before reading");

    assert!(matches!(error, ToolError::InvalidPath { .. }));

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn read_file_lines_tool_returns_typed_error_for_non_utf8_file() {
    let (filesystem, root) = apply_patch_test_filesystem("read-lines-non-utf8").await;
    let path = VirtualPath::try_from("/binary.dat").expect("path is valid");
    filesystem
        .write_file(&path, vec![0xff, 0xfe, 0xfd])
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ReadFileLinesTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("read file lines tool should register");

    let error = registry
        .call(
            &ToolName::from("read_file_lines"),
            ToolInput::new(
                CallId::from("call-read-lines-non-utf8"),
                json!({
                    "path": "binary.dat",
                    "start_line": 1,
                    "line_count": 1
                }),
            ),
        )
        .await
        .expect_err("non-utf8 file should fail");

    assert!(matches!(error, ToolError::InvalidUtf8 { ref path, .. } if path == "binary.dat"));

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn list_dir_tool_returns_sorted_directory_entries_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("list-dir").await;
    let dir = VirtualPath::try_from("/src").expect("path is valid");
    let nested = VirtualPath::try_from("/src/nested").expect("path is valid");
    let lib = VirtualPath::try_from("/src/lib.rs").expect("path is valid");
    filesystem.create_dir(&dir).await.expect("create src dir");
    filesystem
        .create_dir(&nested)
        .await
        .expect("create nested dir");
    filesystem
        .write_file(&lib, b"pub fn ok() {}\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ListDirTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("list dir tool should register");

    let output = registry
        .call(
            &ToolName::from("list_dir"),
            ToolInput::new(
                CallId::from("call-list-dir"),
                json!({
                    "path": "src"
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(
        output.result,
        json!({
            "path": "src",
            "entries": [
                {
                    "path": "src/lib.rs",
                    "file_name": "lib.rs",
                    "file_type": "file"
                },
                {
                    "path": "src/nested",
                    "file_name": "nested",
                    "file_type": "directory"
                }
            ]
        })
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn file_metadata_tool_returns_file_type_and_length_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("metadata").await;
    let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"hello\n".to_vec())
        .await
        .expect("seed file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(FileMetadataTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("file metadata tool should register");

    let output = registry
        .call(
            &ToolName::from("file_metadata"),
            ToolInput::new(
                CallId::from("call-metadata"),
                json!({
                    "path": "notes.txt"
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(
        output.result,
        json!({
            "path": "notes.txt",
            "file_type": "file",
            "len": 6
        })
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn search_text_tool_returns_matches_under_directory_through_registry() {
    let (filesystem, root) = apply_patch_test_filesystem("search-text").await;
    let src = VirtualPath::try_from("/src").expect("path is valid");
    let nested = VirtualPath::try_from("/src/nested").expect("path is valid");
    let lib = VirtualPath::try_from("/src/lib.rs").expect("path is valid");
    let mod_file = VirtualPath::try_from("/src/nested/mod.rs").expect("path is valid");
    let binary = VirtualPath::try_from("/src/blob.bin").expect("path is valid");
    filesystem.create_dir(&src).await.expect("create src dir");
    filesystem
        .create_dir(&nested)
        .await
        .expect("create nested dir");
    filesystem
        .write_file(&lib, b"fn alpha() {}\nfn beta() {}\n".to_vec())
        .await
        .expect("seed lib file");
    filesystem
        .write_file(&mod_file, b"pub fn alpha_nested() {}\n".to_vec())
        .await
        .expect("seed nested file");
    filesystem
        .write_file(&binary, vec![0xff, 0xfe])
        .await
        .expect("seed binary file");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(SearchTextTool::new(filesystem)),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("search text tool should register");

    let output = registry
        .call(
            &ToolName::from("search_text"),
            ToolInput::new(
                CallId::from("call-search-text"),
                json!({
                    "path": "src",
                    "query": "alpha",
                    "max_results": 10
                }),
            ),
        )
        .await
        .expect("tool should run");

    assert_eq!(
        output.result,
        json!({
            "path": "src",
            "query": "alpha",
            "matches": [
                {
                    "path": "src/lib.rs",
                    "line_number": 1,
                    "line_text": "fn alpha() {}",
                    "match_start": 3,
                    "match_end": 8
                },
                {
                    "path": "src/nested/mod.rs",
                    "line_number": 1,
                    "line_text": "pub fn alpha_nested() {}",
                    "match_start": 7,
                    "match_end": 12
                }
            ]
        })
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn registered_echo_tool_can_be_called_through_registry() {
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
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
fn permission_modes_apply_the_declared_matrix() {
    let cases = [
        (
            ToolPermissionMode::Allow,
            ToolKind::Write,
            DangerLevel::High,
            false,
        ),
        (
            ToolPermissionMode::PartialAllow,
            ToolKind::Read,
            DangerLevel::Low,
            false,
        ),
        (
            ToolPermissionMode::PartialAllow,
            ToolKind::Read,
            DangerLevel::Medium,
            true,
        ),
        (
            ToolPermissionMode::PartialAllow,
            ToolKind::Write,
            DangerLevel::Low,
            true,
        ),
        (
            ToolPermissionMode::RequireApproval,
            ToolKind::Read,
            DangerLevel::Low,
            true,
        ),
    ];

    for (mode, kind, danger_level, expected_approval) in cases {
        let mut registry = BuiltinToolRegistry::new(mode);
        registry
            .register(Arc::new(EchoTool::new()), kind, danger_level)
            .expect("echo registers");

        let approval_metadata = registry
            .authorization(&ToolName::from("echo"))
            .expect("echo is registered");

        assert_eq!(approval_metadata.is_some(), expected_approval);
    }
}

#[test]
fn duplicate_registration_fails() {
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("first registration should succeed");

    let error = registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
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
        .register(
            Arc::new(ApplyPatchTool::new(filesystem.clone())),
            ToolKind::Write,
            DangerLevel::High,
        )
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
        .register(
            Arc::new(ApplyPatchTool::new(filesystem.clone())),
            ToolKind::Write,
            DangerLevel::High,
        )
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
        .register(
            Arc::new(ApplyPatchTool::new(filesystem.clone())),
            ToolKind::Write,
            DangerLevel::High,
        )
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
        stratum_filesystem::FilesystemError::NotFound { .. }
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
        .register(
            Arc::new(ApplyPatchTool::new(filesystem.clone())),
            ToolKind::Write,
            DangerLevel::High,
        )
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
        .register(
            Arc::new(ApplyPatchTool::new(filesystem.clone())),
            ToolKind::Write,
            DangerLevel::High,
        )
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
        .register(
            Arc::new(ApplyPatchTool::new(filesystem)),
            ToolKind::Write,
            DangerLevel::High,
        )
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
