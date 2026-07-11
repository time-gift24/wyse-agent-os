//! Shared, strictly validated configuration for Wyse applications.

mod error;

use std::{collections::HashSet, net::SocketAddr, path::PathBuf, str::FromStr, time::Duration};

pub use error::ConfigError;
use serde::{Deserialize, Serialize};
use wyse_core::{ModelId, ToolName};
use wyse_infra::NatsEventStreamBusConfig;

/// Top-level Wyse configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Config {
    /// Agent filesystem configuration.
    pub agent: AgentConfig,
    /// LLM provider configuration.
    pub llm: LlmConfig,
    /// HTTP API configuration, when the API is enabled.
    #[serde(default)]
    pub api: Option<ApiConfig>,
    /// NATS configuration, when the event stream bus is enabled.
    #[serde(default)]
    pub nats: Option<NatsConfig>,
}

/// Agent filesystem configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AgentConfig {
    /// Root directory for persisted agent state.
    pub storage_root: PathBuf,
}

/// HTTP API configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ApiConfig {
    /// Socket address on which the API listens.
    #[serde(default = "default_api_bind")]
    pub bind: SocketAddr,
    /// Browser origins allowed to call the API.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

fn default_api_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8080))
}

/// NATS event stream bus configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NatsConfig {
    /// NATS server URL.
    pub url: String,
    /// JetStream stream name.
    pub stream_name: String,
    /// Subject prefix for agent events.
    pub subject_prefix: String,
    /// Number of stream replicas.
    pub replicas: usize,
    /// Maximum retained event age in seconds.
    pub max_age_seconds: u64,
    /// Maximum retained stream size in bytes.
    pub max_bytes: i64,
    /// Maximum retained event count.
    pub max_messages: i64,
}

/// Stable name used to identify an agent definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct AgentName(String);

impl AgentName {
    /// Returns the validated name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for AgentName {
    type Err = ConfigError;

    /// Parses an ASCII agent name matching `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidAgentName`] if the value is empty, too long, or not
    /// the documented ASCII pattern.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut bytes = value.bytes();
        let valid = value.len() <= 64
            && bytes
                .next()
                .is_some_and(|byte| byte.is_ascii_alphanumeric())
            && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
        if !valid {
            return Err(ConfigError::InvalidAgentName {
                value: value.to_owned(),
            });
        }
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for AgentName {
    type Error = ConfigError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<AgentName> for String {
    fn from(value: AgentName) -> Self {
        value.0
    }
}

/// LLM defaults and supported providers.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct LlmConfig {
    /// Model used when an agent template does not override it.
    pub default: ModelId,
    /// DeepSeek provider configuration.
    #[serde(default)]
    pub deepseek: Option<ProviderConfig>,
    /// OpenAI provider configuration.
    #[serde(default)]
    pub openai: Option<ProviderConfig>,
}

/// Credentials and allowed models for one LLM provider.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ProviderConfig {
    /// Provider API key.
    pub api_key: String,
    /// Provider-local model names available to agents.
    pub models: Vec<String>,
}

/// Validated, self-contained agent definition without provider credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ResolvedAgentDefinition {
    /// Agent name.
    pub agent_name: AgentName,
    /// Selected model.
    pub model: ModelId,
    /// Tools exposed to the agent.
    pub tools: Vec<ToolName>,
    /// System prompt.
    pub prompt: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentTemplate {
    #[serde(default)]
    model: Option<ModelId>,
    #[serde(default)]
    tools: Vec<ToolName>,
    prompt: String,
}

impl Config {
    /// Parses and validates strict TOML configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if TOML decoding or configuration validation fails.
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(input)?;
        config.validate()?;
        Ok(config)
    }

    /// Resolves a strict TOML agent template against this configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the template is invalid or selects an unconfigured model.
    pub fn resolve_template(
        &self,
        agent_name: AgentName,
        input: &str,
    ) -> Result<ResolvedAgentDefinition, ConfigError> {
        let template: AgentTemplate = toml::from_str(input)?;
        let prompt = template.prompt.trim();
        if prompt.is_empty() {
            return Err(ConfigError::EmptyPrompt);
        }
        validate_tools(&template.tools)?;
        let model = template.model.unwrap_or_else(|| self.llm.default.clone());
        self.validate_model_configured(&model)?;

        Ok(ResolvedAgentDefinition {
            agent_name,
            model,
            tools: template.tools,
            prompt: prompt.to_owned(),
        })
    }

    /// Returns the configured API section.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingSection`] when no API section was provided.
    pub fn require_api(&self) -> Result<&ApiConfig, ConfigError> {
        self.api
            .as_ref()
            .ok_or(ConfigError::MissingSection { section: "api" })
    }

    /// Returns the configured NATS section.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::MissingSection`] when no NATS section was provided.
    pub fn require_nats(&self) -> Result<&NatsConfig, ConfigError> {
        self.nats
            .as_ref()
            .ok_or(ConfigError::MissingSection { section: "nats" })
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.agent.storage_root.as_os_str().is_empty() {
            return Err(ConfigError::InvalidStorageRoot);
        }
        validate_provider("deepseek", self.llm.deepseek.as_ref())?;
        validate_provider("openai", self.llm.openai.as_ref())?;
        if let Some(api) = &self.api {
            for origin in &api.allowed_origins {
                if origin == "*" || http::HeaderValue::from_str(origin).is_err() {
                    return Err(ConfigError::InvalidAllowedOrigin);
                }
            }
        }
        self.validate_model_configured(&self.llm.default)
    }

    fn validate_model_configured(&self, model: &ModelId) -> Result<(), ConfigError> {
        let provider = match model.provider_name() {
            "deepseek" => self.llm.deepseek.as_ref(),
            "openai" => self.llm.openai.as_ref(),
            _ => None,
        };
        if provider.is_some_and(|config| {
            config
                .models
                .iter()
                .any(|configured| configured == model.model_name())
        }) {
            return Ok(());
        }
        Err(ConfigError::ModelNotConfigured {
            model: model.clone(),
        })
    }
}

impl TryFrom<&NatsConfig> for NatsEventStreamBusConfig {
    type Error = ConfigError;

    /// Converts validated scalar NATS settings into runtime settings.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::InvalidNatsConfig`] when a required string is blank or a numeric
    /// limit is not positive.
    fn try_from(value: &NatsConfig) -> Result<Self, Self::Error> {
        for (field, candidate) in [
            ("url", value.url.as_str()),
            ("stream_name", value.stream_name.as_str()),
            ("subject_prefix", value.subject_prefix.as_str()),
        ] {
            if candidate.trim().is_empty() {
                return Err(ConfigError::InvalidNatsConfig { field });
            }
        }
        if !(1..=5).contains(&value.replicas) {
            return Err(ConfigError::InvalidNatsConfig { field: "replicas" });
        }
        if value.max_age_seconds == 0 {
            return Err(ConfigError::InvalidNatsConfig {
                field: "max_age_seconds",
            });
        }
        if value.max_bytes <= 0 {
            return Err(ConfigError::InvalidNatsConfig { field: "max_bytes" });
        }
        if value.max_messages <= 0 {
            return Err(ConfigError::InvalidNatsConfig {
                field: "max_messages",
            });
        }

        Ok(Self {
            url: value.url.clone(),
            stream_name: value.stream_name.clone(),
            subject_prefix: value.subject_prefix.clone(),
            replicas: value.replicas,
            max_age: Duration::from_secs(value.max_age_seconds),
            max_bytes: value.max_bytes,
            max_messages: value.max_messages,
        })
    }
}

impl ResolvedAgentDefinition {
    /// Parses and validates a resolved definition from TOML.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when TOML decoding, the prompt, or tool uniqueness is invalid.
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        let mut definition: Self = toml::from_str(input)?;
        let prompt = definition.prompt.trim();
        if prompt.is_empty() {
            return Err(ConfigError::EmptyPrompt);
        }
        validate_tools(&definition.tools)?;
        definition.prompt = prompt.to_owned();
        Ok(definition)
    }

    /// Encodes this resolved definition as TOML.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::TomlEncode`] if TOML serialization fails.
    pub fn encode(&self) -> Result<String, ConfigError> {
        toml::to_string(self).map_err(ConfigError::from)
    }
}

fn validate_provider(
    provider: &'static str,
    config: Option<&ProviderConfig>,
) -> Result<(), ConfigError> {
    let Some(config) = config else {
        return Ok(());
    };
    if config.api_key.trim().is_empty() {
        return Err(ConfigError::EmptyApiKey { provider });
    }
    if config.models.is_empty() {
        return Err(ConfigError::EmptyModels { provider });
    }
    let mut models = HashSet::with_capacity(config.models.len());
    for model in &config.models {
        if !models.insert(model.as_str()) {
            return Err(ConfigError::DuplicateModel {
                provider,
                model: model.clone(),
            });
        }
    }
    Ok(())
}

fn validate_tools(tools: &[ToolName]) -> Result<(), ConfigError> {
    let mut names = HashSet::with_capacity(tools.len());
    for tool in tools {
        if !names.insert(tool.as_str()) {
            return Err(ConfigError::DuplicateTool {
                tool: tool.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::error::Error as StdError;

    use super::{AgentName, Config, ConfigError, ResolvedAgentDefinition};
    use wyse_infra::NatsEventStreamBusConfig;

    const VALID_CONFIG: &str = r#"
[agent]
storage_root = "./.wyse/agents"

[llm]
default = "deepseek:deepseek-v4-flash"

[llm.deepseek]
api_key = "secret-key"
models = ["deepseek-v4-flash", "deepseek-v4-pro"]

[api]
bind = "127.0.0.1:8080"
allowed_origins = ["http://localhost:5173"]

[nats]
url = "nats://127.0.0.1:4222"
stream_name = "AGENT_EVENTS"
subject_prefix = "events.agent"
replicas = 1
max_age_seconds = 604800
max_bytes = 1073741824
max_messages = 1000000
"#;

    const VALID_TEMPLATE_WITHOUT_MODEL: &str = r#"
tools = ["read_file", "apply_patch"]
prompt = "  You are a coding agent.  "
"#;

    #[test]
    fn parses_complete_config() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");

        assert_eq!(
            config.agent.storage_root.to_string_lossy(),
            "./.wyse/agents"
        );
        assert_eq!(config.llm.default.as_str(), "deepseek:deepseek-v4-flash");
        assert_eq!(config.require_api().expect("api exists").bind.port(), 8080);
        assert_eq!(config.require_nats().expect("nats exists").replicas, 1);
    }

    #[test]
    fn api_bind_defaults_to_loopback_when_omitted() {
        let input = VALID_CONFIG.replace("bind = \"127.0.0.1:8080\"\n", "");

        let config = Config::parse(&input).expect("config parses");

        assert_eq!(
            config.require_api().expect("api exists").bind,
            "127.0.0.1:8080".parse().expect("default bind parses")
        );
    }

    #[test]
    fn rejects_wildcard_and_invalid_allowed_origins() {
        for origin in ["*", "bad\norigin"] {
            let input = VALID_CONFIG.replace(
                "allowed_origins = [\"http://localhost:5173\"]",
                &format!("allowed_origins = [{origin:?}]"),
            );

            assert!(matches!(
                Config::parse(&input),
                Err(ConfigError::InvalidAllowedOrigin)
            ));
        }
    }

    #[test]
    fn rejects_unknown_config_field() {
        let input = VALID_CONFIG.replace("[agent]", "[agent]\nunknown = true");
        assert!(matches!(Config::parse(&input), Err(ConfigError::Toml(_))));
    }

    #[test]
    fn malformed_toml_error_redacts_input_from_entire_source_chain() {
        let secret = "malformed-secret-key";
        let input = format!("[agent]\nstorage_root = \"{secret}");
        let error = Config::parse(&input).expect_err("malformed TOML is rejected");

        assert_error_chain_redacts(&error, secret);
    }

    #[test]
    fn unknown_field_error_redacts_input_from_entire_source_chain() {
        let secret = "secret-key";
        let input = VALID_CONFIG.replace("[agent]", "[agent]\nunknown = true");
        let error = Config::parse(&input).expect_err("unknown field is rejected");

        assert_error_chain_redacts(&error, secret);
    }

    #[test]
    fn rejects_duplicate_models() {
        let input = VALID_CONFIG.replace(
            "models = [\"deepseek-v4-flash\", \"deepseek-v4-pro\"]",
            "models = [\"deepseek-v4-flash\", \"deepseek-v4-flash\"]",
        );
        assert!(matches!(
            Config::parse(&input),
            Err(ConfigError::DuplicateModel { .. })
        ));
    }

    #[test]
    fn rejects_default_model_not_in_provider_list() {
        let input = VALID_CONFIG.replace(
            "default = \"deepseek:deepseek-v4-flash\"",
            "default = \"deepseek:not-configured\"",
        );
        assert!(matches!(
            Config::parse(&input),
            Err(ConfigError::ModelNotConfigured { .. })
        ));
    }

    #[test]
    fn rejects_empty_storage_root() {
        let input = VALID_CONFIG.replace("./.wyse/agents", "");
        assert!(matches!(
            Config::parse(&input),
            Err(ConfigError::InvalidStorageRoot)
        ));
    }

    #[test]
    fn rejects_empty_provider_api_key() {
        let input = VALID_CONFIG.replace("api_key = \"secret-key\"", "api_key = \"  \"");
        assert!(matches!(
            Config::parse(&input),
            Err(ConfigError::EmptyApiKey { .. })
        ));
    }

    #[test]
    fn rejects_empty_provider_models() {
        let input = VALID_CONFIG.replace(
            "models = [\"deepseek-v4-flash\", \"deepseek-v4-pro\"]",
            "models = []",
        );
        assert!(matches!(
            Config::parse(&input),
            Err(ConfigError::EmptyModels { .. })
        ));
    }

    #[test]
    fn parses_valid_agent_name() {
        let name: AgentName = "coding-agent-2".parse().expect("name parses");
        assert_eq!(name.as_str(), "coding-agent-2");
    }

    #[test]
    fn agent_name_accepts_uppercase_underscore_and_flexible_hyphens() {
        for value in ["CodingAgent", "coding_agent", "a--b", "coding-"] {
            let name: AgentName = value.parse().expect("name parses");
            assert_eq!(name.as_str(), value);
        }
    }

    #[test]
    fn rejects_invalid_agent_names() {
        for value in ["", "éagent", "_coding", "-coding"] {
            assert!(matches!(
                value.parse::<AgentName>(),
                Err(ConfigError::InvalidAgentName { .. })
            ));
        }
    }

    #[test]
    fn rejects_agent_name_longer_than_64_bytes() {
        let value = "a".repeat(65);
        assert!(matches!(
            value.parse::<AgentName>(),
            Err(ConfigError::InvalidAgentName { .. })
        ));
    }

    #[test]
    fn template_without_model_uses_system_default() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let definition = config
            .resolve_template(name, VALID_TEMPLATE_WITHOUT_MODEL)
            .expect("template resolves");
        assert_eq!(definition.model.as_str(), "deepseek:deepseek-v4-flash");
        assert_eq!(definition.prompt, "You are a coding agent.");
    }

    #[test]
    fn template_model_overrides_system_default() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let template = r#"
model = "deepseek:deepseek-v4-pro"
tools = ["read_file"]
prompt = "Use the requested model."
"#;

        let definition = config
            .resolve_template(name, template)
            .expect("template resolves");
        assert_eq!(definition.model.as_str(), "deepseek:deepseek-v4-pro");
    }

    #[test]
    fn rejects_unconfigured_template_model() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let template = r#"
model = "deepseek:not-configured"
tools = []
prompt = "Use the requested model."
"#;

        assert!(matches!(
            config.resolve_template(name, template),
            Err(ConfigError::ModelNotConfigured { .. })
        ));
    }

    #[test]
    fn rejects_unknown_template_field() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let template = format!("{VALID_TEMPLATE_WITHOUT_MODEL}\nunknown = true");

        assert!(matches!(
            config.resolve_template(name, &template),
            Err(ConfigError::Toml(_))
        ));
    }

    #[test]
    fn rejects_duplicate_template_tools() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let template = r#"
tools = ["read_file", "read_file"]
prompt = "Use tools."
"#;

        assert!(matches!(
            config.resolve_template(name, template),
            Err(ConfigError::DuplicateTool { .. })
        ));
    }

    #[test]
    fn rejects_empty_template_prompt() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let template = "tools = []\nprompt = \"  \"";

        assert!(matches!(
            config.resolve_template(name, template),
            Err(ConfigError::EmptyPrompt)
        ));
    }

    #[test]
    fn missing_optional_sections_are_reported_when_required() {
        let input = VALID_CONFIG
            .split("\n[api]")
            .next()
            .expect("config has api section");
        let config = Config::parse(input).expect("base config parses");

        assert!(matches!(
            config.require_api(),
            Err(ConfigError::MissingSection { section: "api" })
        ));
        assert!(matches!(
            config.require_nats(),
            Err(ConfigError::MissingSection { section: "nats" })
        ));
    }

    #[test]
    fn converts_valid_nats_config() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let nats = NatsEventStreamBusConfig::try_from(config.require_nats().expect("nats exists"))
            .expect("nats converts");

        assert_eq!(nats.max_age.as_secs(), 604800);
        assert_eq!(nats.subject_prefix, "events.agent");
    }

    #[test]
    fn rejects_invalid_nats_config() {
        let input = VALID_CONFIG.replace("replicas = 1", "replicas = 0");
        let config = Config::parse(&input).expect("config parses");

        assert!(matches!(
            NatsEventStreamBusConfig::try_from(config.require_nats().expect("nats exists")),
            Err(ConfigError::InvalidNatsConfig { .. })
        ));
    }

    #[test]
    fn rejects_nats_replica_count_above_runtime_limit() {
        let input = VALID_CONFIG.replace("replicas = 1", "replicas = 6");
        let config = Config::parse(&input).expect("config parses");

        assert!(matches!(
            NatsEventStreamBusConfig::try_from(config.require_nats().expect("nats exists")),
            Err(ConfigError::InvalidNatsConfig { field: "replicas" })
        ));
    }

    #[test]
    fn resolved_definition_round_trips_without_secret() {
        let config = Config::parse(VALID_CONFIG).expect("config parses");
        let name: AgentName = "coding-agent".parse().expect("name parses");
        let definition = config
            .resolve_template(name, VALID_TEMPLATE_WITHOUT_MODEL)
            .expect("template resolves");

        let encoded = definition.encode().expect("definition encodes");
        assert!(!encoded.contains("secret-key"));
        assert_eq!(
            ResolvedAgentDefinition::parse(&encoded).expect("definition parses"),
            definition
        );
    }

    #[test]
    fn validation_error_does_not_contain_api_key() {
        let input = VALID_CONFIG.replace(
            "models = [\"deepseek-v4-flash\", \"deepseek-v4-pro\"]",
            "models = [\"deepseek-v4-flash\", \"deepseek-v4-flash\"]",
        );
        let error = Config::parse(&input).expect_err("duplicate model is rejected");

        assert!(!format!("{error:?}").contains("secret-key"));
        assert!(!error.to_string().contains("secret-key"));
    }

    fn assert_error_chain_redacts(error: &ConfigError, secret: &str) {
        assert!(!format!("{error:?}").contains(secret));
        assert!(!error.to_string().contains(secret));

        let mut source = StdError::source(error);
        while let Some(error) = source {
            assert!(!format!("{error:?}").contains(secret));
            assert!(!error.to_string().contains(secret));
            source = error.source();
        }
    }
}
