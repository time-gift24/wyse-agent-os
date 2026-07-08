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
