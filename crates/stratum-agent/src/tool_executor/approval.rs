//! Approval contract for tool calls that are not pre-authorized.

use async_trait::async_trait;
use serde_json::Value;
use stratum_core::{ApprovalDecision, ApprovalId, CallId, DangerLevel, ToolKind, ToolName};
use tokio_util::sync::CancellationToken;

use super::ToolApprovalError;

/// Complete request supplied to a tool approval policy.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ToolApprovalRequest {
    /// Identity of this approval interaction.
    pub approval_id: ApprovalId,
    /// Provider identity of the tool call.
    pub call_id: CallId,
    /// Provider-visible tool identity.
    pub tool_name: ToolName,
    /// Parsed tool arguments.
    pub arguments: Value,
    /// Whether the tool observes or mutates state.
    pub tool_kind: ToolKind,
    /// Declared danger of the tool.
    pub danger_level: DangerLevel,
}

/// Policy boundary for calls that require approval.
#[async_trait]
pub trait ToolApproval: Send + Sync {
    /// Resolves one approval request.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the approval interaction cannot complete.
    ///
    /// # Cancellation safety
    ///
    /// Implementations must cooperatively observe `cancellation` and return
    /// [`ToolApprovalError::Cancelled`] when it is signalled. Cancellation must not be reported as
    /// a generic interaction failure. [`ToolExecutor`](super::ToolExecutor) continues polling this
    /// future after cancellation rather than dropping it, so implementations need cooperative
    /// cancellation responsiveness; no additional drop-safety contract is imposed here.
    async fn request(
        &self,
        request: ToolApprovalRequest,
        cancellation: &CancellationToken,
    ) -> Result<ApprovalDecision, ToolApprovalError>;
}
