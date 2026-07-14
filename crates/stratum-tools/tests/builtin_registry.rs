use std::sync::Arc;

use serde_json::json;
use stratum_core::{CallId, DangerLevel, ToolKind, ToolName, ToolSpec};
use stratum_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};
use stratum_tools::{
    ApplyPatchTool, BuiltinToolRegistry, EchoTool, FileMetadataTool, ListDirTool,
    ReadFileLinesTool, SearchTextTool, ToolError, ToolInput, ToolPermissionMode, ToolRegistry,
};
use tokio_util::sync::CancellationToken;

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
async fn builtin_validation_rejects_every_deterministic_invalid_input_without_filesystem_work() {
    let (filesystem, root) = apply_patch_test_filesystem("validation").await;
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("echo should register");
    registry
        .register(
            Arc::new(ReadFileLinesTool::new(filesystem.clone())),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("read file lines should register");
    registry
        .register(
            Arc::new(ListDirTool::new(filesystem.clone())),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("list dir should register");
    registry
        .register(
            Arc::new(FileMetadataTool::new(filesystem.clone())),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("file metadata should register");
    registry
        .register(
            Arc::new(SearchTextTool::new(filesystem.clone())),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("search text should register");
    registry
        .register(
            Arc::new(ApplyPatchTool::new(filesystem)),
            ToolKind::Write,
            DangerLevel::High,
        )
        .expect("apply patch should register");

    let cases = [
        ("echo", json!(42), "invalid argument arguments"),
        (
            "read_file_lines",
            json!({"path": "notes.txt", "start_line": 1}),
            "invalid tool input",
        ),
        (
            "read_file_lines",
            json!({"path": "notes.txt", "start_line": 0, "line_count": 1}),
            "invalid tool input",
        ),
        (
            "read_file_lines",
            json!({"path": "../secret", "start_line": 1, "line_count": 1}),
            "invalid path",
        ),
        ("list_dir", json!({}), "invalid tool input"),
        ("list_dir", json!({"path": "../secret"}), "invalid path"),
        ("file_metadata", json!({}), "invalid tool input"),
        (
            "file_metadata",
            json!({"path": "../secret"}),
            "invalid path",
        ),
        (
            "search_text",
            json!({"path": "src", "query": ""}),
            "invalid argument query",
        ),
        (
            "search_text",
            json!({"path": "src", "query": "needle", "max_results": 0}),
            "invalid tool input",
        ),
        (
            "search_text",
            json!({"path": "../secret", "query": "needle"}),
            "invalid path",
        ),
        ("apply_patch", json!({}), "invalid tool input"),
        (
            "apply_patch",
            json!({"operation": {"type": "rename_file", "path": "notes.txt"}}),
            "invalid tool operation",
        ),
        (
            "apply_patch",
            json!({"operation": {"type": "delete_file", "path": "../secret"}}),
            "invalid path",
        ),
        (
            "apply_patch",
            json!({"operation": {"type": "create_file", "path": "notes.txt"}}),
            "invalid argument diff",
        ),
        (
            "apply_patch",
            json!({"operation": {"type": "update_file", "path": "notes.txt"}}),
            "invalid argument diff",
        ),
    ];

    for (name, arguments, expected_prefix) in cases {
        let input = ToolInput::new(CallId::from("call-invalid"), arguments);
        let validation_error = registry
            .validate(&ToolName::from(name), &input)
            .expect_err("invalid input should fail synchronous validation");
        assert!(
            validation_error.to_string().starts_with(expected_prefix),
            "unexpected {name} validation error: {validation_error}"
        );
        let call_error = registry
            .call(&ToolName::from(name), input, &CancellationToken::new())
            .await
            .expect_err("direct calls must revalidate the same invalid input");
        assert_eq!(
            rejection_class(&validation_error),
            rejection_class(&call_error),
            "validation and call rejection classes differ for {name}"
        );
        assert_eq!(
            validation_error.to_string(),
            call_error.to_string(),
            "validation and call rejection messages differ for {name}"
        );
    }
    assert!(
        tokio::fs::read_dir(&root)
            .await
            .expect("validation root should remain readable")
            .next_entry()
            .await
            .expect("reading validation root should succeed")
            .is_none(),
        "synchronous validation must not perform filesystem work"
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

fn rejection_class(error: &ToolError) -> &'static str {
    match error {
        ToolError::InvalidInput { .. } => "invalid_input",
        ToolError::InvalidOperation { .. } => "invalid_operation",
        ToolError::InvalidPath { .. } => "invalid_path",
        ToolError::InvalidArgument { .. } => "invalid_argument",
        _ => "unexpected",
    }
}

#[tokio::test]
async fn apply_patch_schema_requires_diff_only_for_create_and_update() {
    let (filesystem, root) = apply_patch_test_filesystem("schema").await;
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ApplyPatchTool::new(filesystem)),
            ToolKind::Write,
            DangerLevel::High,
        )
        .expect("apply patch should register");

    let spec = registry
        .specs()
        .into_iter()
        .find(|spec| spec.name == ToolName::from("apply_patch"))
        .expect("apply patch spec should be registered");
    let operation_schema = &spec.input_schema["properties"]["operation"];

    assert_eq!(operation_schema["required"], json!(["type", "path"]));
    assert_eq!(
        operation_schema["allOf"],
        json!([{
            "if": {
                "properties": {
                    "type": {"enum": ["create_file", "update_file"]}
                },
                "required": ["type"]
            },
            "then": {"required": ["diff"]}
        }])
    );

    let _ = tokio::fs::remove_dir_all(root).await;
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
        )
        .await
        .expect_err("missing tool should fail");

    assert!(matches!(
        error,
        ToolError::ToolNotFound { ref name } if name == &ToolName::from("missing")
    ));
}

#[tokio::test]
async fn cancelled_apply_patch_preserves_existing_content_and_version() {
    let (filesystem, root) = apply_patch_test_filesystem("cancelled-update").await;
    let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
    filesystem
        .write_file(&path, b"original\n".to_vec())
        .await
        .expect("seed file");
    let before = filesystem
        .get(&path)
        .await
        .expect("get should succeed")
        .expect("seeded file should exist");
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(ApplyPatchTool::new(filesystem.clone())),
            ToolKind::Write,
            DangerLevel::High,
        )
        .expect("apply patch tool should register");
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = registry
        .call(
            &ToolName::from("apply_patch"),
            ToolInput::new(
                CallId::from("call-cancelled-update"),
                json!({
                    "operation": {
                        "type": "update_file",
                        "path": "notes.txt",
                        "diff": "@@\n-original\n+changed\n"
                    }
                }),
            ),
            &cancellation,
        )
        .await
        .expect_err("cancelled patch should not run");

    assert!(matches!(error, ToolError::Cancelled));
    let after = filesystem
        .get(&path)
        .await
        .expect("get should succeed")
        .expect("cancelled patch should preserve the file");
    assert_eq!(after, before);

    let _ = tokio::fs::remove_dir_all(root).await;
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
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
            &CancellationToken::new(),
        )
        .await
        .expect_err("invalid path should fail before patching");

    assert!(matches!(error, ToolError::InvalidPath { .. }));

    let _ = tokio::fs::remove_dir_all(root).await;
}
