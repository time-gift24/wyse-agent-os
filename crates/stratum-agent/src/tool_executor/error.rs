//! Typed failures that prevent safe tool execution.

use std::error::Error as StdError;

use stratum_infra::DurableEventSinkError;
use thiserror::Error;

/// Failure returned by a tool approval policy.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ToolApprovalError {
    /// The approval interaction was cancelled.
    #[error("tool approval cancelled")]
    Cancelled,
    /// An approval interaction backend failed.
    #[error("tool approval interaction failed")]
    Interaction {
        /// Backend failure source, retained without exposing it in the top-level message.
        #[source]
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
}

impl ToolApprovalError {
    /// Wraps an approval interaction backend failure while preserving its source chain.
    #[must_use]
    pub fn interaction(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Interaction {
            source: Box::new(source),
        }
    }
}

/// Failure that prevents the tool executor from preserving its durable ordering.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ToolExecutorError {
    /// A required pre-execution event was not acknowledged.
    #[error("durable tool event was not acknowledged")]
    Durability {
        /// Durable event sink failure.
        #[source]
        source: DurableEventSinkError,
    },
    /// The approval interaction failed.
    #[error("tool approval failed")]
    Approval {
        /// Approval failure source.
        #[source]
        source: ToolApprovalError,
    },
    /// A newer approval decision is not understood safely.
    #[error("unsupported tool approval decision")]
    UnsupportedApprovalDecision,
}

impl From<DurableEventSinkError> for ToolExecutorError {
    fn from(source: DurableEventSinkError) -> Self {
        Self::Durability { source }
    }
}

impl From<ToolApprovalError> for ToolExecutorError {
    fn from(source: ToolApprovalError) -> Self {
        Self::Approval { source }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn durability_conversion_preserves_typed_source() {
        let error = ToolExecutorError::from(DurableEventSinkError::UnsupportedEvent {
            event_type: "future_event",
        });

        assert!(matches!(&error, ToolExecutorError::Durability { .. }));
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<DurableEventSinkError>()),
            Some(DurableEventSinkError::UnsupportedEvent {
                event_type: "future_event"
            })
        ));
    }

    #[test]
    fn approval_conversion_preserves_typed_source() {
        let error = ToolExecutorError::from(ToolApprovalError::Cancelled);

        assert!(matches!(&error, ToolExecutorError::Approval { .. }));
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<ToolApprovalError>()),
            Some(ToolApprovalError::Cancelled)
        ));
    }
}
