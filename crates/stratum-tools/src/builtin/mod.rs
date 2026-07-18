//! Builtin tool implementations.

mod apply_patch;
mod file_metadata;
mod filesystem;
mod list_dir;
mod read_file_lines;
mod search_text;

use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use serde_json::json;
use stratum_core::{DangerLevel, ToolKind, ToolName, ToolSpec};
use tokio_util::sync::CancellationToken;

use crate::{Tool, ToolError, ToolInput, ToolOutput, ToolPermissionMode, ToolRegistry};

pub use apply_patch::ApplyPatchTool;
pub use file_metadata::FileMetadataTool;
pub use list_dir::ListDirTool;
pub use read_file_lines::ReadFileLinesTool;
pub use search_text::SearchTextTool;

/// Registry backed by builtin in-memory tools.
struct RegisteredTool {
    tool: Arc<dyn Tool>,
    tool_kind: ToolKind,
    danger_level: DangerLevel,
}

pub struct BuiltinToolRegistry {
    tools: BTreeMap<ToolName, RegisteredTool>,
    permission_mode: ToolPermissionMode,
}

impl BuiltinToolRegistry {
    /// Creates a builtin registry with the requested permission behavior.
    #[must_use]
    pub fn new(permission_mode: ToolPermissionMode) -> Self {
        Self {
            tools: BTreeMap::new(),
            permission_mode,
        }
    }
}

impl Default for BuiltinToolRegistry {
    fn default() -> Self {
        Self::new(ToolPermissionMode::Allow)
    }
}

#[async_trait]
impl ToolRegistry for BuiltinToolRegistry {
    fn register(
        &mut self,
        tool: Arc<dyn Tool>,
        tool_kind: ToolKind,
        danger_level: DangerLevel,
    ) -> Result<(), ToolError> {
        let name = tool.spec().name.clone();
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateTool { name });
        }

        self.tools.insert(
            name,
            RegisteredTool {
                tool,
                tool_kind,
                danger_level,
            },
        );
        Ok(())
    }

    fn authorization(&self, name: &ToolName) -> Result<Option<(ToolKind, DangerLevel)>, ToolError> {
        let registered = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::ToolNotFound { name: name.clone() })?;
        let allowed = match self.permission_mode {
            ToolPermissionMode::Allow => true,
            ToolPermissionMode::PartialAllow => {
                registered.tool_kind == ToolKind::Read
                    && registered.danger_level == DangerLevel::Low
            }
            ToolPermissionMode::RequireApproval => false,
        };
        Ok((!allowed).then_some((registered.tool_kind, registered.danger_level)))
    }

    fn validate(&self, name: &ToolName, input: &ToolInput) -> Result<(), ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::ToolNotFound { name: name.clone() })?;
        tool.validate(input)
    }

    fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>> {
        self.tools
            .get(name)
            .map(|registered| Arc::clone(&registered.tool))
    }

    fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .values()
            .map(|registered| registered.tool.spec().clone())
            .collect()
    }

    async fn call(
        &self,
        name: &ToolName,
        input: ToolInput,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .get(name)
            .ok_or_else(|| ToolError::ToolNotFound { name: name.clone() })?;

        tool.call(input, cancellation).await
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

    fn validate(&self, input: &ToolInput) -> Result<(), ToolError> {
        if input.arguments.is_object() {
            Ok(())
        } else {
            Err(ToolError::InvalidArgument {
                name: "arguments",
                reason: "must be an object",
            })
        }
    }

    async fn call(
        &self,
        input: ToolInput,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        self.validate(&input)?;
        if cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        Ok(ToolOutput::new(input.arguments))
    }
}
