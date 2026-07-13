//! Registry for injected LLM providers.

use std::{collections::BTreeMap, sync::Arc};

use stratum_core::{ModelConfig, ModelId};

use crate::{ConfigurableLlmProvider, LlmError, LlmProvider, ModelDescriptor};

/// Registry of LLM providers injected by the application.
#[derive(Default)]
pub struct LlmProviderManager {
    providers: BTreeMap<ModelId, Arc<dyn ConfigurableLlmProvider>>,
}

impl LlmProviderManager {
    /// Creates an empty provider registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers an injected provider by its bound model id.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::DuplicateProvider`] when a provider is already registered for the model.
    pub fn register(&mut self, provider: Arc<dyn ConfigurableLlmProvider>) -> Result<(), LlmError> {
        let model = provider.model_id();
        if self.providers.contains_key(&model) {
            return Err(LlmError::DuplicateProvider { model });
        }
        self.providers.insert(model, provider);
        Ok(())
    }

    /// Returns the provider registered for a model.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::ProviderNotFound`] when no provider is registered for the model.
    pub fn get(&self, model: &ModelId) -> Result<Arc<dyn ConfigurableLlmProvider>, LlmError> {
        self.providers
            .get(model)
            .cloned()
            .ok_or_else(|| LlmError::ProviderNotFound {
                model: model.clone(),
            })
    }

    /// Configures the provider registered for a model configuration.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::ProviderNotFound`] when no provider is registered for the model, or
    /// [`LlmError::InvalidModelParameters`] when the provider rejects the parameters.
    pub fn configure(&self, config: &ModelConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
        self.get(&config.model)?.configure(&config.parameters)
    }

    /// Returns the default configuration for a registered model.
    ///
    /// # Errors
    ///
    /// Returns [`LlmError::ProviderNotFound`] when no provider is registered for the model.
    pub fn default_model_config(&self, model: &ModelId) -> Result<ModelConfig, LlmError> {
        Ok(self.get(model)?.default_model_config())
    }

    /// Lists registered models in deterministic model-id order.
    #[must_use]
    pub fn models(&self) -> Vec<ModelDescriptor> {
        self.providers
            .iter()
            .map(|(model, provider)| ModelDescriptor {
                model: model.clone(),
                parameters_schema: provider.parameter_schema(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use stratum_core::ModelId;

    use serde_json::{Map, json};
    use stratum_core::ModelConfig;

    use crate::{
        ChatRequest, ChatResponse, ChatStream, ConfigurableLlmProvider, LlmError, LlmProvider,
        LlmProviderManager,
    };

    #[derive(Debug)]
    struct TestProvider;

    #[async_trait]
    impl LlmProvider for TestProvider {
        fn model_id(&self) -> ModelId {
            ModelId::new("test", "model").expect("static model id is valid")
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
            Err(LlmError::MockExhausted)
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
            Err(LlmError::MockExhausted)
        }
    }

    impl ConfigurableLlmProvider for TestProvider {
        fn parameter_schema(&self) -> serde_json::Value {
            json!({"type": "object", "additionalProperties": false, "default": {}})
        }

        fn default_model_config(&self) -> ModelConfig {
            ModelConfig::new(self.model_id(), Map::new())
        }

        fn configure(
            &self,
            parameters: &Map<String, serde_json::Value>,
        ) -> Result<Arc<dyn LlmProvider>, LlmError> {
            if parameters.is_empty() {
                Ok(Arc::new(Self))
            } else {
                Err(LlmError::InvalidModelParameters {
                    model: self.model_id(),
                })
            }
        }
    }

    #[test]
    fn registered_provider_can_be_looked_up() {
        let provider: Arc<dyn ConfigurableLlmProvider> = Arc::new(TestProvider);
        let model = provider.model_id();
        let mut manager = LlmProviderManager::new();

        manager
            .register(Arc::clone(&provider))
            .expect("provider should register");

        assert!(Arc::ptr_eq(
            &provider,
            &manager.get(&model).expect("provider should be found")
        ));
    }

    #[test]
    fn duplicate_registration_returns_duplicate_provider_error() {
        let mut manager = LlmProviderManager::new();
        manager
            .register(Arc::new(TestProvider))
            .expect("provider should register");

        let error = manager
            .register(Arc::new(TestProvider))
            .expect_err("duplicate provider should fail");

        assert!(matches!(error, LlmError::DuplicateProvider { .. }));
    }

    #[test]
    fn missing_provider_returns_provider_not_found_error() {
        let manager = LlmProviderManager::new();
        let model = ModelId::new("test", "model").expect("static model id is valid");

        let error = match manager.get(&model) {
            Err(error) => error,
            Ok(_) => panic!("missing provider should fail"),
        };

        assert!(matches!(error, LlmError::ProviderNotFound { .. }));
    }

    #[test]
    fn manager_configures_registered_provider_with_model_parameters() {
        let mut manager = LlmProviderManager::new();
        let config = ModelConfig::new(
            ModelId::new("test", "model").expect("static model id is valid"),
            Map::new(),
        );
        manager
            .register(Arc::new(TestProvider))
            .expect("provider should register");

        let configured = manager
            .configure(&config)
            .expect("registered provider should configure");

        assert_eq!(configured.model_id(), config.model);
    }

    #[test]
    fn manager_returns_registered_default_model_config() {
        let mut manager = LlmProviderManager::new();
        let model = ModelId::new("test", "model").expect("static model id is valid");
        manager
            .register(Arc::new(TestProvider))
            .expect("provider should register");

        assert_eq!(
            manager
                .default_model_config(&model)
                .expect("registered provider should have defaults"),
            ModelConfig::new(model, Map::new())
        );
    }

    #[test]
    fn manager_lists_registered_models_with_parameter_schemas() {
        let mut manager = LlmProviderManager::new();
        manager
            .register(Arc::new(TestProvider))
            .expect("provider should register");

        let models = manager.models();

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model.as_str(), "test:model");
        assert_eq!(models[0].parameters_schema["default"], json!({}));
    }
}
