//! Public tool traits and execution types.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use wyse_core::{CallId, DangerLevel, ToolKind, ToolName, ToolSpec};

use crate::ToolError;

/// Registry-wide tool permission behavior.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolPermissionMode {
    #[default]
    Allow,
    PartialAllow,
    RequireApproval,
}

/// Result of authorizing one registered tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolAuthorization {
    Allowed,
    RequireApproval {
        tool_kind: ToolKind,
        danger_level: DangerLevel,
    },
}

impl ToolAuthorization {
    /// Returns metadata when the tool call requires approval.
    #[must_use]
    pub const fn approval_metadata(self) -> Option<(ToolKind, DangerLevel)> {
        match self {
            Self::Allowed => None,
            Self::RequireApproval {
                tool_kind,
                danger_level,
            } => Some((tool_kind, danger_level)),
        }
    }
}

/// Input passed to one runtime tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolInput {
    /// Provider call identity.
    pub call_id: CallId,
    /// Parsed tool arguments.
    pub arguments: Value,
}

impl ToolInput {
    /// Creates tool input from a provider call id and parsed arguments.
    #[must_use]
    pub const fn new(call_id: CallId, arguments: Value) -> Self {
        Self { call_id, arguments }
    }
}

/// Output returned by one runtime tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolOutput {
    /// Tool result payload.
    pub result: Value,
}

impl ToolOutput {
    /// Creates tool output from a result payload.
    #[must_use]
    pub const fn new(result: Value) -> Self {
        Self { result }
    }
}

/// Runtime tool implementation.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the provider-visible tool specification.
    fn spec(&self) -> &ToolSpec;

    /// Executes the tool.
    ///
    /// # Errors
    ///
    /// Returns a tool error when execution fails.
    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError>;
}

/// Registry of pre-populated runtime tools.
#[async_trait]
pub trait ToolRegistry: Send + Sync {
    /// Registers a tool by its provider-visible name.
    ///
    /// # Errors
    ///
    /// Returns an error when another tool with the same name is already registered.
    fn register(
        &mut self,
        tool: Arc<dyn Tool>,
        tool_kind: ToolKind,
        danger_level: DangerLevel,
    ) -> Result<(), ToolError>;

    /// Returns the authorization result for a registered tool.
    ///
    /// # Errors
    ///
    /// Returns an error when the tool is not registered.
    fn authorization(&self, name: &ToolName) -> Result<ToolAuthorization, ToolError>;

    /// Returns a registered tool by name.
    fn get(&self, name: &ToolName) -> Option<Arc<dyn Tool>>;

    /// Returns provider-visible specs for all registered tools.
    fn specs(&self) -> Vec<ToolSpec>;

    /// Executes a registered tool by name.
    ///
    /// # Errors
    ///
    /// Returns an error when the tool is missing or execution fails.
    async fn call(&self, name: &ToolName, input: ToolInput) -> Result<ToolOutput, ToolError>;
}
