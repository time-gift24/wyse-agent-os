//! Agent runtime loop for Stratum.

pub mod definition;
pub mod error;

pub(crate) mod r#loop;

pub use definition::{Agent, AgentBuilder, AgentConfig};
pub use error::AgentError;
