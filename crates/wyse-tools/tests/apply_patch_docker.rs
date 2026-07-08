use std::sync::Arc;

use serde_json::json;
use wyse_core::CallId;
use wyse_filesystem::{
    Filesystem, FilesystemError, LocalFilesystem, LocalFilesystemConfig, VirtualPath,
};
use wyse_tools::{ApplyPatchTool, Tool, ToolInput};

const LINE_COUNT: usize = 1_050;
const HEAD_COUNT: usize = 10;
const PATCH_START: usize = 401;
const PATCH_END: usize = 700;
const CONTEXT_BEFORE: usize = PATCH_START - 1;
const CONTEXT_AFTER: usize = PATCH_END + 1;

#[ignore = "crate integration test"]
#[tokio::test]
async fn apply_patch_tool_updates_and_deletes_large_file_in_docker_sandbox() {
    let root = std::env::var_os("WYSE_TOOLS_DOCKER_SANDBOX")
        .map(std::path::PathBuf::from)
        .expect("WYSE_TOOLS_DOCKER_SANDBOX must point at the compose-mounted sandbox");
    assert!(
        root.join(".container-ready").is_file(),
        "compose service must write the readiness marker"
    );
    let _ = tokio::fs::remove_file(root.join("large.txt")).await;
    let filesystem = Arc::new(
        LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(128 * 1024),
        })
        .expect("filesystem is valid"),
    );
    let tool = ApplyPatchTool::new(filesystem.clone());
    let path = VirtualPath::try_from("/large.txt").expect("path is valid");

    filesystem
        .write_file(&path, original_file().into_bytes())
        .await
        .expect("seed large file");

    let original = filesystem.read_file(&path).await.expect("read large file");
    let original = String::from_utf8(original).expect("large file is utf-8");
    assert_head_is_original(&original);

    let update = tool
        .call(ToolInput::new(
            CallId::from("call-update"),
            json!({
                "operation": {
                    "type": "update_file",
                    "path": "large.txt",
                    "diff": update_diff()
                }
            }),
        ))
        .await
        .expect("update call should run");
    assert_eq!(update.result["status"], "completed");
    assert_eq!(update.result["output"], "updated large.txt");

    let updated = filesystem
        .read_file(&path)
        .await
        .expect("read updated file");
    let updated = String::from_utf8(updated).expect("updated file is utf-8");
    assert_large_file_patched(&updated);

    let delete = tool
        .call(ToolInput::new(
            CallId::from("call-delete"),
            json!({
                "operation": {
                    "type": "delete_file",
                    "path": "large.txt"
                }
            }),
        ))
        .await
        .expect("delete call should run");
    assert_eq!(delete.result["status"], "completed");
    assert_eq!(delete.result["output"], "deleted large.txt");

    let missing = filesystem
        .read_file(&path)
        .await
        .expect_err("deleted file should be missing");
    assert!(
        matches!(&missing, FilesystemError::NotFound { .. }),
        "expected not found error, got {missing}"
    );
}

fn original_file() -> String {
    let mut content = String::with_capacity(LINE_COUNT * 21);
    for line in 1..=LINE_COUNT {
        content.push_str(&original_line(line));
        content.push('\n');
    }
    content
}

fn update_diff() -> String {
    let mut diff = String::with_capacity((PATCH_END - PATCH_START + 3) * 44);
    diff.push_str("@@\n");
    diff.push_str(&format!(" {}\n", original_line(CONTEXT_BEFORE).trim_end()));
    for line in PATCH_START..=PATCH_END {
        diff.push_str(&format!("-{}\n", original_line(line).trim_end()));
    }
    for line in PATCH_START..=PATCH_END {
        diff.push_str(&format!("+{}\n", patched_line(line).trim_end()));
    }
    diff.push_str(&format!(" {}\n", original_line(CONTEXT_AFTER).trim_end()));
    diff
}

fn assert_head_is_original(content: &str) {
    let head: Vec<_> = content.lines().take(HEAD_COUNT).collect();
    let expected: Vec<_> = (1..=HEAD_COUNT)
        .map(original_line)
        .map(|line| line.trim_end().to_owned())
        .collect();
    assert_eq!(head, expected);
}

fn assert_large_file_patched(content: &str) {
    let lines: Vec<_> = content.lines().collect();
    assert_eq!(lines.len(), LINE_COUNT);
    assert_head_is_original(content);

    assert_eq!(
        lines[CONTEXT_BEFORE - 1],
        original_line(CONTEXT_BEFORE).trim_end()
    );
    assert_eq!(
        lines[CONTEXT_AFTER - 1],
        original_line(CONTEXT_AFTER).trim_end()
    );
    assert_eq!(lines[LINE_COUNT - 1], original_line(LINE_COUNT).trim_end());

    for line in PATCH_START..=PATCH_END {
        assert_eq!(lines[line - 1], patched_line(line).trim_end());
    }
}

fn original_line(line: usize) -> String {
    format!("line {line:04}: original")
}

fn patched_line(line: usize) -> String {
    format!("line {line:04}: patched")
}
