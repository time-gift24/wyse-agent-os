//! Durable, approval-aware tool execution for the agent loop.

mod allow_all;
mod approval;
mod definition;
mod deny_all;
mod error;

pub use allow_all::AllowAllToolApproval;
pub use approval::{ToolApproval, ToolApprovalRequest};
pub use definition::{ToolExecutionOutcome, ToolExecutor};
pub use deny_all::DenyAllToolApproval;
pub use error::{ToolApprovalError, ToolExecutorError};
