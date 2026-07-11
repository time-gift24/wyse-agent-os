//! HTTP-specific error mapping.

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        rejection::{JsonRejection, PathRejection, QueryRejection},
    },
    http::{HeaderValue, StatusCode, header::LOCATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use wyse_agent::AgentError;
use wyse_config::{AgentName, ConfigError};
use wyse_core::{
    AgentId, ApprovalDecision, ApprovalId, ChatMessage, HistoryPage, HistoryQuery, RunId,
    TokenUsage, TurnId,
};
use wyse_infra::EventStreamBusError;
use wyse_store::{AgentState, AgentStatus, StoreError};

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

/// Public projection of one persisted agent state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentView {
    /// Agent identity.
    pub agent_id: AgentId,
    /// Resolved template name.
    pub agent_name: String,
    /// Persisted runtime status.
    pub status: AgentStatus,
    /// Current run identity, when present.
    pub run_id: Option<RunId>,
    /// Current turn identity, when present.
    pub turn_id: Option<TurnId>,
    /// Cumulative model token usage.
    pub usage: TokenUsage,
    /// Last committed complete-message sequence.
    pub last_seq: u64,
    /// Last persisted state update.
    pub updated_at: DateTime<Utc>,
}

/// Response returned after a run is durably accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunAccepted {
    /// Accepted run identity.
    pub run_id: RunId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAgentRequest {
    agent_name: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MessageRequest {
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovalRequest {
    decision: ApprovalDecision,
}

#[derive(Deserialize)]
struct HistoryParams {
    #[serde(default)]
    after_seq: u64,
    through_seq: Option<u64>,
    #[serde(default = "default_history_limit")]
    limit: usize,
}

const fn default_history_limit() -> usize {
    100
}

/// Builds the HTTP API router for one host state.
pub fn router(state: Arc<HostState>) -> Router {
    Router::new()
        .route("/v1/agents", post(create_agent))
        .route("/v1/agents/{agent_id}", get(get_agent))
        .route(
            "/v1/agents/{agent_id}/messages",
            get(get_messages).post(post_message),
        )
        .route("/v1/agents/{agent_id}/resume", post(resume_agent))
        .route("/v1/agents/{agent_id}/cancel", post(cancel_agent))
        .route(
            "/v1/agents/{agent_id}/approvals/{approval_id}",
            post(resolve_approval),
        )
        .with_state(state)
}

async fn create_agent(
    State(state): State<Arc<HostState>>,
    request: Result<Json<CreateAgentRequest>, JsonRejection>,
) -> Result<Response, HostError> {
    let Json(request) = request.map_err(|_| HostError::InvalidRequest)?;
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

async fn get_agent(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
) -> Result<Json<AgentView>, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    let hosted = find_agent(&state, agent_id)?;
    let persisted = hosted.store.load_agent().await?;
    Ok(Json(persisted.into()))
}

async fn get_messages(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
    query: Result<Query<HistoryParams>, QueryRejection>,
) -> Result<Json<HistoryPage>, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    let Query(query) = query.map_err(|_| HostError::InvalidHistoryQuery)?;
    let hosted = find_agent(&state, agent_id)?;
    let page = hosted
        .store
        .history_page(HistoryQuery {
            after_seq: query.after_seq,
            through_seq: query.through_seq,
            limit: query.limit,
        })
        .await?;
    Ok(Json(page))
}

async fn post_message(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
    request: Result<Json<MessageRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<RunAccepted>), HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    let Json(request) = request.map_err(|_| HostError::InvalidRequest)?;
    if request.text.trim().is_empty() {
        return Err(HostError::InvalidMessage);
    }
    let hosted = find_agent(&state, agent_id)?;
    if hosted.needs_resume() {
        return Err(HostError::ResumeRequired);
    }
    let run_id = match hosted.agent.run_turn(ChatMessage::user(request.text)).await {
        Ok(run_id) => run_id,
        Err(error @ AgentError::RunAlreadyActive) => return Err(error.into()),
        Err(error) => {
            hosted.mark_needs_resume();
            return Err(error.into());
        }
    };
    Ok((StatusCode::ACCEPTED, Json(RunAccepted { run_id })))
}

async fn resume_agent(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
) -> Result<(StatusCode, Json<RunAccepted>), HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    let hosted = find_agent(&state, agent_id)?;
    let persisted = hosted.store.load_agent().await?;
    if persisted.status != AgentStatus::Running {
        hosted.clear_needs_resume();
        return Err(AgentError::ResumeNotRunning {
            actual: persisted.status,
        }
        .into());
    }
    let run_id = match hosted.agent.resume().await {
        Ok(run_id) => run_id,
        Err(error @ AgentError::ResumeNotRunning { .. }) => {
            hosted.clear_needs_resume();
            return Err(error.into());
        }
        Err(error) => return Err(error.into()),
    };
    hosted.clear_needs_resume();
    Ok((StatusCode::ACCEPTED, Json(RunAccepted { run_id })))
}

async fn cancel_agent(
    State(state): State<Arc<HostState>>,
    path: Result<Path<AgentId>, PathRejection>,
) -> Result<StatusCode, HostError> {
    let Path(agent_id) = path.map_err(|_| HostError::InvalidRequest)?;
    let hosted = find_agent(&state, agent_id)?;
    if hosted.needs_resume() {
        return Err(HostError::ResumeRequired);
    }
    hosted.agent.stop();
    Ok(StatusCode::ACCEPTED)
}

async fn resolve_approval(
    State(state): State<Arc<HostState>>,
    path: Result<Path<(AgentId, ApprovalId)>, PathRejection>,
    request: Result<Json<ApprovalRequest>, JsonRejection>,
) -> Result<StatusCode, HostError> {
    let Path((agent_id, approval_id)) = path.map_err(|_| HostError::InvalidRequest)?;
    let Json(request) = request.map_err(|_| HostError::InvalidRequest)?;
    let hosted = find_agent(&state, agent_id)?;
    hosted
        .agent
        .resolve_tool_approval(approval_id, request.decision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn find_agent(state: &HostState, agent_id: AgentId) -> Result<Arc<crate::HostedAgent>, HostError> {
    state
        .agent(agent_id)
        .ok_or(HostError::AgentNotFound { agent_id })
}

impl From<AgentState> for AgentView {
    fn from(state: AgentState) -> Self {
        Self {
            agent_id: state.agent_id,
            agent_name: state.name,
            status: state.status,
            run_id: state.run_id,
            turn_id: state.turn_id,
            usage: state.usage,
            last_seq: state.last_seq,
            updated_at: state.updated_at,
        }
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for HostError {
    fn into_response(self) -> Response {
        let (status, code, message) = error_response(&self);
        (
            status,
            Json(ErrorResponse {
                error: ErrorBody { code, message },
            }),
        )
            .into_response()
    }
}

fn error_response(error: &HostError) -> (StatusCode, &'static str, &'static str) {
    match error {
        HostError::AgentNotFound { .. } => (
            StatusCode::NOT_FOUND,
            "agent_not_found",
            "agent was not found",
        ),
        HostError::TemplateNotFound { .. } => (
            StatusCode::NOT_FOUND,
            "agent_template_not_found",
            "agent template was not found",
        ),
        HostError::InvalidMessage => (
            StatusCode::BAD_REQUEST,
            "invalid_message",
            "message text must not be blank",
        ),
        HostError::InvalidRequest => (
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "request is invalid",
        ),
        HostError::InvalidHistoryQuery => (
            StatusCode::BAD_REQUEST,
            "invalid_history_query",
            "history query is invalid",
        ),
        HostError::ResumeRequired => (
            StatusCode::CONFLICT,
            "resume_required",
            "agent has an unfinished persisted turn",
        ),
        HostError::EmptyText => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_agent_template",
            "initial agent text must not be blank",
        ),
        HostError::ToolNotAvailable { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "tool_not_available",
            "agent template requests an unavailable tool",
        ),
        HostError::Config(ConfigError::InvalidAgentName { .. }) => (
            StatusCode::BAD_REQUEST,
            "invalid_agent_name",
            "agent name is invalid",
        ),
        HostError::Config(ConfigError::ModelNotConfigured { .. }) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "model_not_configured",
            "agent model is not configured",
        ),
        HostError::Config(_) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_agent_template",
            "agent template is invalid",
        ),
        HostError::Agent(AgentError::RunAlreadyActive) => (
            StatusCode::CONFLICT,
            "agent_busy",
            "agent already has an active run",
        ),
        HostError::Agent(AgentError::ResumeNotRunning { .. }) => (
            StatusCode::CONFLICT,
            "resume_not_running",
            "agent has no persisted running turn",
        ),
        HostError::Agent(AgentError::NoActiveTurn | AgentError::ApprovalNotFound { .. }) => (
            StatusCode::CONFLICT,
            "approval_not_active",
            "tool approval is not active",
        ),
        HostError::Agent(AgentError::MissingBuilderField { .. })
        | HostError::Llm(_)
        | HostError::Tool(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "agent_initialization_failed",
            "agent initialization failed",
        ),
        HostError::Store(source) | HostError::Agent(AgentError::Store { source }) => {
            store_error_response(source)
        }
        HostError::Agent(AgentError::EventBus {
            source: EventStreamBusError::Persistence { source },
        }) => source
            .downcast_ref::<StoreError>()
            .map_or_else(internal_error_response, store_error_response),
        HostError::EventStreamBus(EventStreamBusError::CursorExpired { .. }) => (
            StatusCode::GONE,
            "cursor_expired",
            "event cursor is no longer retained",
        ),
        HostError::EventStreamBus(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "event_stream_unavailable",
            "event stream is unavailable",
        ),
        _ => internal_error_response(),
    }
}

fn store_error_response(error: &StoreError) -> (StatusCode, &'static str, &'static str) {
    match error {
        StoreError::InvalidHistoryLimit { .. }
        | StoreError::InvalidHistoryRange { .. }
        | StoreError::HistoryBarrierBeyondLast { .. } => (
            StatusCode::BAD_REQUEST,
            "invalid_history_query",
            "history query is invalid",
        ),
        StoreError::Filesystem(source) => filesystem_store_error_response(source),
        StoreError::CasTimeout | StoreError::CasRetriesExhausted => (
            StatusCode::SERVICE_UNAVAILABLE,
            "store_unavailable",
            "agent store is unavailable",
        ),
        _ => internal_error_response(),
    }
}

fn filesystem_store_error_response(
    error: &wyse_filesystem::FilesystemError,
) -> (StatusCode, &'static str, &'static str) {
    match error {
        wyse_filesystem::FilesystemError::PermissionDenied { .. }
        | wyse_filesystem::FilesystemError::LocalIo { .. } => (
            StatusCode::SERVICE_UNAVAILABLE,
            "store_unavailable",
            "agent store is unavailable",
        ),
        _ => internal_error_response(),
    }
}

fn internal_error_response() -> (StatusCode, &'static str, &'static str) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal server error",
    )
}
