//! Scoped output ports for foundational agent-loop events.

mod definition;
mod error;
mod scoped;

pub use definition::{DurableEventSink, TelemetryEventSink};
pub use error::DurableEventSinkError;
pub use scoped::ScopedAgentEventSink;
