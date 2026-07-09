//! Checkpoint persistence primitives for Wyse runtimes.

mod definition;
mod error;

pub use definition::{
    CheckpointId, CheckpointKind, CheckpointRecord, CheckpointStatus, CheckpointStore,
};
pub use error::CheckpointError;
