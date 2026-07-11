use tokio::sync::oneshot;
use wyse_core::{ApprovalDecision, ApprovalId};

use crate::AgentError;

pub(crate) enum TurnCommand {
    ResolveToolApproval {
        approval_id: ApprovalId,
        decision: ApprovalDecision,
        response: oneshot::Sender<Result<(), AgentError>>,
    },
}

pub(crate) fn reject_inactive_command(command: Option<TurnCommand>) -> Result<(), AgentError> {
    let Some(TurnCommand::ResolveToolApproval {
        approval_id,
        response,
        ..
    }) = command
    else {
        return Err(AgentError::TurnCommandClosed);
    };
    let _ = response.send(Err(AgentError::ApprovalNotFound { approval_id }));
    Ok(())
}
