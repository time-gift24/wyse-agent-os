//! HTTP API host for persisted Wyse agents.

mod api;
mod error;
mod host;

pub use api::{AgentCreated, router};
pub use error::HostError;
pub use host::{CreatedAgent, HostState, HostedAgent};
