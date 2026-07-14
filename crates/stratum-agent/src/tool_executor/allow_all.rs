//! Non-interactive approval policy that permits every request.

use async_trait::async_trait;
use stratum_core::ApprovalDecision;
use tokio_util::sync::CancellationToken;

use super::{ToolApproval, ToolApprovalError, ToolApprovalRequest};

/// Non-interactive policy that approves every requested tool call.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAllToolApproval;

#[async_trait]
impl ToolApproval for AllowAllToolApproval {
    async fn request(
        &self,
        _request: ToolApprovalRequest,
        _cancellation: &CancellationToken,
    ) -> Result<ApprovalDecision, ToolApprovalError> {
        Ok(ApprovalDecision::Approve)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use stratum_core::{ApprovalId, CallId, DangerLevel, ToolKind, ToolName};

    use super::*;

    #[tokio::test]
    async fn allow_all_policy_approves_request() {
        let request = ToolApprovalRequest {
            approval_id: ApprovalId::new(),
            call_id: CallId::new("call-1"),
            tool_name: ToolName::new("write_file"),
            arguments: json!({"path": "notes.txt"}),
            tool_kind: ToolKind::Write,
            danger_level: DangerLevel::Medium,
        };

        let decision = AllowAllToolApproval
            .request(request, &CancellationToken::new())
            .await
            .expect("allow-all policy is infallible");

        assert_eq!(decision, ApprovalDecision::Approve);
    }
}
