//! Registry for injected LLM providers.

use std::{collections::BTreeMap, sync::Arc};

use wyse_core::ModelId;

use crate::{LlmError, LlmProvider};

/// Registry of LLM providers injected by the application.
#[derive(Default)]
pub struct LlmProviderManager {
    providers: BTreeMap<ModelId, Arc<dyn LlmProvider>>,
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
    pub fn register(&mut self, provider: Arc<dyn LlmProvider>) -> Result<(), LlmError> {
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
    pub fn get(&self, model: &ModelId) -> Result<Arc<dyn LlmProvider>, LlmError> {
        self.providers
            .get(model)
            .cloned()
            .ok_or_else(|| LlmError::ProviderNotFound {
                model: model.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use wyse_core::ModelId;

    use crate::{ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, LlmProviderManager};

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

    #[test]
    fn registered_provider_can_be_looked_up() {
        let provider: Arc<dyn LlmProvider> = Arc::new(TestProvider);
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
}
