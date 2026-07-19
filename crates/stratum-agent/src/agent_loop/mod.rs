//! Foundational types and errors for the agent loop kernel.

mod error;
mod hook;
mod runner;
mod stream;
mod types;

pub use error::{
    AgentLoopBuildError, AgentLoopError, AgentLoopHookError, AgentLoopHookStage, ProtocolError,
};
pub use hook::{
    AgentLoopHook, IterationHookContext, LlmCallHookContext, LlmCallOutput, LlmErrorAction,
    ToolCallHookContext,
};
pub use runner::{AgentLoop, AgentLoopBuilder};
pub use types::{LoopContext, LoopLimits, LoopOutcome};
