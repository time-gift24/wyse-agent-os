//! HTTP-specific error mapping.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header::LOCATION},
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use wyse_config::{AgentName, ConfigError};
use wyse_core::{AgentId, RunId};

use crate::{HostError, HostState};

/// Response returned after an agent and its initial run are durably accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentCreated {
    /// New agent identity.
    pub agent_id: AgentId,
    /// Resolved template name.
    pub agent_name: String,
    /// Initial run identity.
    pub run_id: RunId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentRequest {
    agent_name: String,
    text: String,
}

/// Builds the HTTP API router for one host state.
pub fn router(state: Arc<HostState>) -> Router {
    Router::new()
        .route("/v1/agents", post(create_agent))
        .with_state(state)
}

async fn create_agent(
    State(state): State<Arc<HostState>>,
    Json(request): Json<CreateAgentRequest>,
) -> Result<Response, HostError> {
    let agent_name: AgentName = request.agent_name.parse()?;
    let created = state.create_agent(agent_name, request.text).await?;
    let location = HeaderValue::from_str(&format!("/v1/agents/{}", created.agent_id))
        .expect("agent id always produces a valid location header");
    let body = AgentCreated {
        agent_id: created.agent_id,
        agent_name: created.agent_name.into(),
        run_id: created.run_id,
    };
    let mut response = (StatusCode::CREATED, Json(body)).into_response();
    response.headers_mut().insert(LOCATION, location);
    Ok(response)
}

impl IntoResponse for HostError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::AgentNotFound { .. } | Self::TemplateNotFound { .. } => StatusCode::NOT_FOUND,
            Self::EmptyText
            | Self::ToolNotAvailable { .. }
            | Self::Config(ConfigError::InvalidAgentName { .. }) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        status.into_response()
    }
}
