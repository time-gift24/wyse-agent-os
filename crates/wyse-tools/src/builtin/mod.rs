//! Builtin tool implementations.

mod apply_patch;

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use serde_json::json;
use wyse_core::{ToolName, ToolSpec};

use crate::{Tool, ToolError, ToolInput, ToolOutput, ToolRegistry};

pub use apply_patch::ApplyPatchTool;

/// Registry backed by builtin in-memory tools.
#[derive(Default)]
pub struct BuiltinToolRegistry {
    tools: BTreeMap<ToolName, Arc<dyn Tool>>,
}

#[async_trait]
impl ToolRegistry for BuiltinToolRegistry {
    fn register(&mut self, tool: Arc<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.spec().name.clone();
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateTool { name });
        }

        self.tools.insert(name, tool);
        Ok(())
    }

    fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|tool| tool.spec().clone())
            .collect()
    }

    async fn call(&self, name: &ToolName, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::ToolNotFound { name: name.clone() })?;

        tool.call(input).await
    }
}

/// Builtin tool that returns its input arguments.
pub struct EchoTool {
    spec: ToolSpec,
}

impl EchoTool {
    /// Creates an echo tool.
    #[must_use]
    pub fn new() -> Self {
        Self {
            spec: ToolSpec::builder()
                .name("echo")
                .description("returns input arguments")
                .input_schema(json!({"type": "object"}))
                .build(),
        }
    }
}

impl Default for EchoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        Ok(ToolOutput::new(input.arguments))
    }
}
