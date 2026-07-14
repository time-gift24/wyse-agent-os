//! Agent runtime loop for Stratum.

pub mod agent_loop;
pub mod definition;
pub mod error;
pub mod tool_executor;

pub(crate) mod r#loop;

pub use agent_loop::{
    AgentLoop, AgentLoopBuildError, AgentLoopBuilder, AgentLoopError, LoopContext, LoopLimit,
    LoopLimits, LoopOutcome, ProtocolError,
};
pub use definition::{Agent, AgentBuilder, AgentConfig};
pub use error::AgentError;
pub use tool_executor::{
    AllowAllToolApproval, DenyAllToolApproval, ToolApproval, ToolApprovalError,
    ToolApprovalRequest, ToolExecutionOutcome, ToolExecutor, ToolExecutorError,
};
