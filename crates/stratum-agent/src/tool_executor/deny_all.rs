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
        cancellation: &CancellationToken,
    ) -> Result<ApprovalDecision, ToolApprovalError> {
        if cancellation.is_cancelled() {
            Err(ToolApprovalError::Cancelled)
        } else {
            Ok(ApprovalDecision::Reject)
        }
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

    #[tokio::test]
    async fn deny_all_policy_honors_pre_cancellation() {
        let request = ToolApprovalRequest {
            approval_id: ApprovalId::new(),
            call_id: CallId::new("call-cancelled"),
            tool_name: ToolName::new("write_file"),
            arguments: json!({"path": "notes.txt"}),
            tool_kind: ToolKind::Write,
            danger_level: DangerLevel::Medium,
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = DenyAllToolApproval
            .request(request, &cancellation)
            .await
            .expect_err("pre-cancellation must prevent a rejection decision");

        assert!(matches!(error, ToolApprovalError::Cancelled));
    }
}
