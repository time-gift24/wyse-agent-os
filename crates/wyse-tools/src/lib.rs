//! Runtime tool abstractions and builtin tools for Wyse.

pub mod builtin;
pub mod definition;
pub mod error;

pub use builtin::{ApplyPatchTool, BuiltinToolRegistry, EchoTool};
pub use definition::{Tool, ToolInput, ToolOutput, ToolRegistry};
pub use error::ToolError;
