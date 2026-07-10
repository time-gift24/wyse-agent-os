//! Built-in agent wiring and executable entry points.

pub mod error;

mod default_agent;

pub use default_agent::build_default_agent;
pub use error::DefaultAgentError;
