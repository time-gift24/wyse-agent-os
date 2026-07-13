//! Structured output request types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Structured output format requested from a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum StructuredOutput {
    /// Request any valid JSON object.
    JsonObject,
    /// Request JSON matching a named schema.
    JsonSchema {
        /// Schema name.
        name: String,
        /// JSON schema document.
        schema: Value,
        /// Whether the provider should enforce the schema strictly.
        strict: bool,
    },
}
