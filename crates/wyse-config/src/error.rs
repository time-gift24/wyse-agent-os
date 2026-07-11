//! Configuration errors.

use thiserror::Error;
use wyse_core::ModelId;

/// Error returned while parsing or validating Wyse configuration.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// TOML input could not be decoded.
    #[error("invalid TOML configuration")]
    Toml(#[source] toml::de::Error),
    /// A resolved definition could not be encoded as TOML.
    #[error("could not encode TOML definition")]
    TomlEncode(#[source] toml::ser::Error),
    /// An agent name did not match the required ASCII pattern.
    #[error("invalid agent name `{value}`")]
    InvalidAgentName { value: String },
    /// The agent storage root was empty.
    #[error("agent storage root must not be empty")]
    InvalidStorageRoot,
    /// A template prompt was empty after trimming.
    #[error("agent prompt must not be empty")]
    EmptyPrompt,
    /// A template listed the same tool more than once.
    #[error("duplicate tool `{tool}`")]
    DuplicateTool { tool: String },
    /// A provider API key was empty.
    #[error("api key for provider `{provider}` must not be empty")]
    EmptyApiKey { provider: &'static str },
    /// A provider had no configured models.
    #[error("provider `{provider}` must configure at least one model")]
    EmptyModels { provider: &'static str },
    /// A provider listed the same model more than once.
    #[error("duplicate model `{model}` for provider `{provider}`")]
    DuplicateModel {
        provider: &'static str,
        model: String,
    },
    /// A selected model was absent from its provider configuration.
    #[error("model `{model}` is not configured")]
    ModelNotConfigured { model: ModelId },
    /// A caller required an optional section that was not configured.
    #[error("missing required configuration section `{section}`")]
    MissingSection { section: &'static str },
    /// A NATS field was not valid for the event stream bus.
    #[error("invalid nats configuration field `{field}`")]
    InvalidNatsConfig { field: &'static str },
}

impl From<toml::de::Error> for ConfigError {
    fn from(mut source: toml::de::Error) -> Self {
        source.set_input(None);
        Self::Toml(source)
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(source: toml::ser::Error) -> Self {
        Self::TomlEncode(source)
    }
}
