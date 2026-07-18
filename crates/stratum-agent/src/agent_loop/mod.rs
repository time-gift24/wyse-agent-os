//! Foundational types and errors for the agent loop kernel.

mod error;
mod runner;
mod stream;
mod types;

pub use error::{AgentLoopBuildError, AgentLoopError, ProtocolError};
pub use runner::{AgentLoop, AgentLoopBuilder};
pub use types::{LoopContext, LoopLimits, LoopOutcome};
