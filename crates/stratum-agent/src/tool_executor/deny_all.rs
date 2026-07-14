//! Non-interactive approval policy that rejects every request.

use async_trait::async_trait;
use stratum_core::ApprovalDecision;
use tokio_util::sync::CancellationToken;

use super::{ToolApproval, ToolApprovalError, ToolApprovalRequest};

/// Non-interactive policy that rejects every requested tool call.
#[derive(Debug, Clone, Copy, Default)]
pub struct DenyAllToolApproval;

#[async_trait]
impl ToolApproval for DenyAllToolApproval {
    async fn request(
        &self,
        _request: ToolApprovalRequest,
        _cancellation: &CancellationToken,
    ) -> Result<ApprovalDecision, ToolApprovalError> {
        Ok(ApprovalDecision::Reject)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use stratum_core::{ApprovalId, CallId, DangerLevel, ToolKind, ToolName};

    use super::*;

    #[tokio::test]
    async fn deny_all_policy_rejects_request() {
        let request = ToolApprovalRequest {
            approval_id: ApprovalId::new(),
            call_id: CallId::new("call-1"),
            tool_name: ToolName::new("write_file"),
            arguments: json!({"path": "notes.txt"}),
            tool_kind: ToolKind::Write,
            danger_level: DangerLevel::Medium,
        };

        let decision = DenyAllToolApproval
            .request(request, &CancellationToken::new())
            .await
            .expect("deny-all policy is infallible");

        assert_eq!(decision, ApprovalDecision::Reject);
    }
}
