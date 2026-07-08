use std::sync::Arc;

use serde_json::json;
use wyse_core::{CallId, ToolName, ToolSpec};
use wyse_tools::{BuiltinToolRegistry, EchoTool, ToolError, ToolInput, ToolRegistry};

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
